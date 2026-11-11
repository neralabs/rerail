#!/usr/bin/env python3
"""Compare the ifn join shapes over captured integer relations, without changing the capture.

Run under nice -n 19. Example capture mappings are recorded in the integration journal.
Compilation, loading, and index creation are excluded from the query CPU measurements.
"""
import argparse
import hashlib
import json
import re
import sqlite3
import statistics
import time
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    for name in ("fv", "pv", "icall"):
        parser.add_argument("--" + name, required=True, help="Capture table name")
    parser.add_argument("--fn-tag", type=int, required=True)
    parser.add_argument("--inst-tag", type=int, required=True)
    parser.add_argument("--iterations", type=int, default=7)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("iterations must be positive")
    for name in (args.fv, args.pv, args.icall):
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
            parser.error("capture table names must be SQL identifiers")
    db = sqlite3.connect(":memory:", cached_statements=64)
    db.execute("ATTACH DATABASE ? AS cap", (args.capture.resolve().as_uri() + "?mode=ro",))
    tables = {"fv": args.fv, "pv": args.pv, "icall": args.icall}
    for table, source in tables.items():
        db.execute(f'CREATE TABLE {table} AS SELECT * FROM cap."{source}"')
    for statement in (
        "CREATE INDEX fv_kind ON fv(c2)",
        "CREATE INDEX fv_key ON fv(c0,c1,c2)",
        "CREATE INDEX pv_key ON pv(c2,c1)",
        "CREATE INDEX pv_obj ON pv(c0,c1,c2)",
        "CREATE INDEX icall_name ON icall(c2)",
        "CREATE INDEX icall_objname ON icall(c1,c2)",
    ):
        db.execute(statement)
    fn, inst = args.fn_tag, args.inst_tag
    queries = {
        "fv_pv_icall": f"SELECT DISTINCT i.c0,f.c3 FROM fv f CROSS JOIN pv p ON p.c2=f.c0 AND p.c1={inst} CROSS JOIN icall i ON i.c1=p.c0 AND i.c2=f.c1 WHERE f.c2={fn}",
        "fv_icall_pv": f"SELECT DISTINCT i.c0,f.c3 FROM fv f CROSS JOIN icall i ON i.c2=f.c1 CROSS JOIN pv p ON p.c0=i.c1 AND p.c1={inst} AND p.c2=f.c0 WHERE f.c2={fn}",
        "icall_pv_fv": f"SELECT DISTINCT i.c0,f.c3 FROM icall i CROSS JOIN pv p ON p.c0=i.c1 AND p.c1={inst} CROSS JOIN fv f ON f.c0=p.c2 AND f.c1=i.c2 AND f.c2={fn}",
        "demand_keys": f"""WITH demand AS MATERIALIZED (SELECT DISTINCT c0 AS k,c1 AS a FROM fv WHERE c2={fn}),
          wanted AS MATERIALIZED (SELECT i.c0 AS c,p.c2 AS k,i.c2 AS a FROM icall i CROSS JOIN pv p ON p.c0=i.c1 AND p.c1={inst}
          WHERE EXISTS (SELECT 1 FROM demand d WHERE d.k=p.c2 AND d.a=i.c2))
          SELECT DISTINCT w.c,f.c3 FROM wanted w CROSS JOIN fv f ON f.c0=w.k AND f.c1=w.a AND f.c2={fn}""",
    }
    expected = None
    results = []
    for shape, sql in queries.items():
        # Prepare and warm the statement before measuring repeated execution.
        rows = sorted(db.execute(sql).fetchall())
        if expected is None:
            expected = rows
        assert rows == expected, (shape, "output mismatch")
        samples = []
        for _ in range(args.iterations):
            vm = [0]

            def progress():
                vm[0] += 100
                return 0

            db.set_progress_handler(progress, 100)
            start = time.process_time_ns()
            got = db.execute(sql).fetchall()
            cpu = time.process_time_ns() - start
            db.set_progress_handler(None, 0)
            assert sorted(got) == expected
            samples.append({"cpuNs": cpu, "vmStepsRoundedDownTo100": vm[0]})
        result = {"shape": shape, "rows": len(rows), "sql": sql, "samples": samples,
                  "medianCpuNs": statistics.median(s["cpuNs"] for s in samples),
                  "plan": db.execute("EXPLAIN QUERY PLAN " + sql).fetchall()}
        results.append(result)
        print(shape, result["medianCpuNs"], "CPU ns", samples[-1]["vmStepsRoundedDownTo100"], "VM", len(rows), "rows")
    # An empty captured result alone cannot detect a rewrite that always returns nothing.
    # Add one isolated positive match in memory, and one call with the wrong name.
    used = set()
    for table in tables:
        used.update(v for row in db.execute(f"SELECT * FROM {table}") for v in row)
    marker = -9_000_000_000_000_000_000
    assert all(marker + i not in used for i in range(8))
    k, a, f, o, c, wrong = (marker + i for i in range(6))
    db.execute("INSERT INTO fv VALUES (?,?,?,?)", (k, a, fn, f))
    db.execute("INSERT INTO pv VALUES (?,?,?)", (o, inst, k))
    db.executemany("INSERT INTO icall VALUES (?,?,?)", [(c, o, a), (wrong, o, wrong)])
    for shape, sql in queries.items():
        assert sorted(db.execute(sql).fetchall()) == sorted(expected + [(c, f)]), (shape, "positive-control mismatch")
    report = {"capture": str(args.capture.resolve()), "captureSha256": hashlib.sha256(args.capture.read_bytes()).hexdigest(),
              "tables": tables, "fnTag": fn, "instTag": inst, "sqlite": sqlite3.sqlite_version,
              "positiveControl": "one matching call and one rejected call; all shapes agree", "results": results}
    args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
