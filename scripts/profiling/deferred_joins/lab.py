#!/usr/bin/env python3
"""Replay connected joins over controlled inputs or a captured PostHog stratum."""

import argparse
import json
import os
from pathlib import Path
import resource
import sqlite3
import statistics
import subprocess
import sys
import time

from cases import CASES, MODES, digest, factors, oracle, reader_keys, schedule
from strategies import Strategy


def connect(case, capture):
    if capture:
        db = sqlite3.connect(Path(capture).resolve().as_uri() + "?mode=ro", uri=True)
    else:
        db = sqlite3.connect(":memory:")
        for table, col, rows in zip(("l", "r"), ("a", "b"), factors(case)):
            db.execute(
                f"CREATE TABLE {table}({col} INTEGER,k INTEGER,PRIMARY KEY({col},k)) WITHOUT ROWID"
            )
            db.executemany(f"INSERT INTO {table} VALUES (?,?)", sorted(rows))
            db.execute(f"CREATE INDEX key_{table} ON {table}(k,{col})")
        db.execute("ANALYZE")
    db.execute("PRAGMA hard_heap_limit=268435456")
    db.execute("PRAGMA temp_store=FILE")
    db.execute("PRAGMA cache_size=-12000")
    return db


def profile_shape(db):
    sql = """WITH lhs AS (SELECT k,count(*) n FROM l GROUP BY k),
                  rhs AS (SELECT k,count(*) n FROM r GROUP BY k)
             SELECT (SELECT coalesce(sum(n),0) FROM lhs), (SELECT coalesce(sum(n),0) FROM rhs),
                    coalesce(sum(lhs.n*rhs.n),0),coalesce(max(lhs.n*rhs.n),0) FROM lhs JOIN rhs USING(k)"""
    return dict(
        zip(
            ("left_rows", "right_rows", "join_witnesses", "largest_key_product"),
            db.execute(sql).fetchone(),
        )
    )


def worker(case, capture, mode, cache_limit):
    db = connect(case, capture)
    implementation = Strategy(db, mode, not capture, cache_limit)
    rounds = []
    answers = {}
    for label, keys, edits in schedule(case, db):
        keys = reader_keys(case, label, keys, answers)
        steps = 0
        start = time.monotonic()

        def progress():
            nonlocal steps
            steps += 1000
            return int(time.monotonic() - start > 45)

        db.set_progress_handler(progress, 1000)
        cpu = time.process_time()
        if edits:
            implementation.invalidate()
            for op, b, k in edits:
                if op == "delete":
                    db.execute("DELETE FROM r WHERE b=? AND k=?", (b, k))
                else:
                    db.execute("INSERT INTO r VALUES (?,?)", (b, k))
        answers, metrics = implementation.run(keys)
        cpu_ms = (time.process_time() - cpu) * 1000
        wall_ms = (time.monotonic() - start) * 1000
        db.set_progress_handler(None, 0)
        consume = time.process_time()
        signature = digest(answers, keys)
        consume_ms = (time.process_time() - consume) * 1000
        rounds.append(
            dict(
                phase=label,
                cpu_ms=round(cpu_ms, 3),
                wall_ms=round(wall_ms, 3),
                vm_steps_approx=steps,
                **metrics,
                consume_cpu_ms=round(consume_ms, 3),
                **signature,
            )
        )
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return dict(
        rounds=rounds,
        peak_rss_mib=round(peak / (1048576 if sys.platform == "darwin" else 1024), 2),
    )


def run(args):
    if args.repeats < 1 or args.cache_keys_rows < 0 or args.max_witnesses < 1:
        raise ValueError(
            "use positive repeats and witness limit, and a nonnegative cache budget"
        )
    experiments = [(case, None) for case in args.cases]
    experiments.extend(("posthog", str(p.resolve())) for p in args.capture)
    if args.coverage:
        experiments.extend(("coverage", str(p.resolve())) for p in args.capture)
    if not experiments:
        raise ValueError("select a synthetic case or a capture")
    report = dict(
        load=os.getloadavg(),
        sqlite=sqlite3.sqlite_version,
        python=sys.version.split()[0],
        repeats=args.repeats,
        cache_keys_rows=args.cache_keys_rows,
        max_eager_witnesses=args.max_witnesses,
        cases=[],
    )
    for case, capture in experiments:
        db = connect(case, capture)
        shape = profile_shape(db)
        if case == "coverage" and shape["join_witnesses"] > args.max_witnesses:
            db.close()
            report["cases"].append(
                dict(
                    name=case,
                    capture=capture,
                    shape=shape,
                    skipped="coverage exceeds witness budget",
                )
            )
            continue
        expected = oracle(case, db)
        db.close()
        result = dict(
            name=case, capture=capture, shape=shape, expected=expected, runs=[]
        )
        if capture:
            manifest = Path(capture).with_name("manifest.json")
            if manifest.is_file():
                result["corpus"] = json.loads(manifest.read_text())
        report["cases"].append(result)
        for rep in range(args.repeats):
            for mode in (
                args.modes[rep % len(args.modes) :]
                + args.modes[: rep % len(args.modes)]
            ):
                if mode == "eager" and shape["join_witnesses"] > args.max_witnesses:
                    result["eager_skip"] = "witness budget"
                    continue
                cmd = [
                    sys.executable,
                    __file__,
                    "--worker",
                    case,
                    mode,
                    "--cache-keys-rows",
                    str(args.cache_keys_rows),
                ]
                if capture:
                    cmd.extend(["--capture", capture])
                sample = json.loads(subprocess.check_output(cmd, timeout=75))
                actual = {
                    r["phase"]: {k: r[k] for k in ("rows", "sha256")}
                    for r in sample["rounds"]
                }
                if actual != expected:
                    raise AssertionError(
                        f"{case} {mode} differs: {actual} expected {expected}"
                    )
                result["runs"].append(dict(mode=mode, repeat=rep + 1, **sample))
        print(
            case,
            Path(capture).parent.name if capture else "",
            shape,
            {
                m: round(
                    statistics.median(
                        r["rounds"][0]["cpu_ms"]
                        for r in result["runs"]
                        if r["mode"] == m
                    ),
                    3,
                )
                for m in MODES
                if any(r["mode"] == m for r in result["runs"])
            },
            flush=True,
        )
    with args.out.open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--cases", nargs="*", choices=CASES, default=CASES)
    p.add_argument("--modes", nargs="+", choices=MODES, default=MODES)
    p.add_argument("--capture", type=Path, action="append", default=[])
    p.add_argument(
        "--coverage",
        action="store_true",
        help="also stress the captured inputs with projected and full reads",
    )
    p.add_argument("--repeats", type=int, default=3)
    p.add_argument("--cache-keys-rows", type=int, default=8192)
    p.add_argument("--max-witnesses", type=int, default=1000000)
    p.add_argument(
        "--out", type=Path, default=Path("join-results.json"), help="new JSON report"
    )
    p.add_argument("--worker", nargs=2, help=argparse.SUPPRESS)
    args = p.parse_args()
    if args.worker:
        case, mode = args.worker
        print(
            json.dumps(
                worker(
                    case,
                    args.capture[0] if args.capture else None,
                    mode,
                    args.cache_keys_rows,
                )
            )
        )
    else:
        if args.out.exists():
            p.error(f"output already exists: {args.out}")
        os.nice(19)
        run(args)


if __name__ == "__main__":
    main()
