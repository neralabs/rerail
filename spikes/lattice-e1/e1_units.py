#!/usr/bin/env python3
"""Work-unit balance of the push kernel (`round::parallel`) per round, from the columns alone.

    e1_units.py GRAPH_DIR [THREADS...]

A round's frontier is the set of ids at that level (Y2's levels = the fixpoint's); a work unit is
a range of `ceil(words / (16 T))` frontier words, as `chunk_words` cuts them. Per round: the
edges in the heaviest unit against the ideal per-worker share (edges / T). When the heaviest unit
exceeds the share, the round's critical path is that unit, whatever the thread count.
"""
import array
import sys
from pathlib import Path


def col(p, t="I"):
    a = array.array(t)
    a.frombytes(Path(p).read_bytes())
    return a


def main():
    g = Path(sys.argv[1])
    threads = [int(x) for x in sys.argv[2:]] or [6, 12]
    off = col(g / "base_off.u32")
    ids, lv = col(g / "y2_id.u32"), col(g / "y2_lvl.u32")
    n = len(off) - 1
    words = (n + 63) // 64
    rounds = max(lv) + 1
    for t in threads:
        cw = -(-words // (t * 16))
        span = cw * 64
        units = -(-words // cw)
        print(f"threads {t}: {units} units of {span} ids")
        print("| round | edges | heaviest unit | share of round | ideal per worker | heaviest / ideal |")
        print("|---|---|---|---|---|---|")
        for k in range(rounds):
            per = [0] * units
            for i, l in zip(ids, lv):
                if l == k:
                    per[i // span] += off[i + 1] - off[i]
            tot = sum(per)
            top = max(per)
            ideal = tot / t
            print(f"| {k} | {tot} | {top} | {top / tot:.2f} | {ideal:.0f} | {top / ideal:.2f} |")
        print()


if __name__ == "__main__":
    main()
