"""Check a join-size bound against a saved capture without enumerating any pairs."""

import argparse
import heapq
import itertools
import json
import math
import os
from pathlib import Path
import sqlite3
import time


def aligned(left, right):
    """Merge two streams of (integer key, degree), each in ascending key order."""
    lhs = ((key, degree, 0) for key, degree in left)
    rhs = ((key, 0, degree) for key, degree in right)
    for key, rows in itertools.groupby(heapq.merge(lhs, rhs), key=lambda row: row[0]):
        left_count = right_count = 0
        for _, a, b in rows:
            left_count += a
            right_count += b
        yield key, left_count, right_count


def bounds(left, right, top):
    if top < 0:
        raise ValueError("top must be nonnegative")
    rows = [0, 0]
    squares = [0, 0]
    maxima = [0, 0]
    heavy = [[], []]
    actual = 0
    for key, left_count, right_count in aligned(left, right):
        # Oracle only. The bound below cannot read this value.
        actual += left_count * right_count
        for side, n in enumerate((left_count, right_count)):
            rows[side] += n
            squares[side] += n * n
            maxima[side] = max(maxima[side], n)
            if top and n:
                # Carry both degrees to simulate exact opposite-side posting counts.
                item = (n, -key, left_count, right_count)
                if len(heavy[side]) < top:
                    heapq.heappush(heavy[side], item)
                else:
                    heapq.heappushpop(heavy[side], item)

    known = {
        -negative_key: (left_count, right_count)
        for side in heavy
        for _, negative_key, left_count, right_count in side
    }
    lower = sum(left_count * right_count for left_count, right_count in known.values())
    remaining = []
    for side in (0, 1):
        degrees = [pair[side] for pair in known.values()]
        mass = rows[side] - sum(degrees)
        moment = squares[side] - sum(n * n for n in degrees)
        # Every unlisted degree is at most the smallest retained top degree.
        maximum = heavy[side][0][0] if heavy[side] else maxima[side]
        remaining.append((mass, moment, maximum))
    (lm, ls, lmax), (rm, rs, rmax) = remaining
    upper = lower + min(lm * rmax, rm * lmax, math.isqrt(ls * rs))
    return dict(
        left_rows=rows[0],
        right_rows=rows[1],
        counted_keys=len(known),
        lower_witnesses=lower,
        upper_witnesses=upper,
        oracle_witnesses=actual,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument(
        "--top", type=int, choices=range(129), default=64, metavar="0..128"
    )
    args = parser.parse_args()
    start = time.monotonic()
    with sqlite3.connect(args.capture.resolve().as_uri() + "?mode=ro", uri=True) as db:
        db.execute("PRAGMA cache_size=-8192")
        db.execute("PRAGMA temp_store=FILE")
        db.execute("PRAGMA threads=1")
        db.set_progress_handler(lambda: time.monotonic() - start > 15, 8000)
        result = bounds(
            db.execute("SELECT k,count(*) FROM l GROUP BY k ORDER BY k"),
            db.execute("SELECT k,count(*) FROM r GROUP BY k ORDER BY k"),
            args.top,
        )
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    os.nice(19)
    main()
