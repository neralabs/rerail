#!/usr/bin/env python3
"""Replay captured native SQL inputs; compare all rule rows across graph adapters."""
import argparse
import hashlib
import itertools
import json
from pathlib import Path
import platform
import os
import resource
import sqlite3
import sys
import time

from reachability_sql import connect, materialize, quote, requested_roots


def resolve(value, options):
    if isinstance(value, str):
        return value
    key = options[value["$option"]]
    key = str(key).lower() if isinstance(key, bool) else str(key)
    return resolve(value.get("cases", {}).get(key, value.get("default")), options)


def normalized(rows):
    return sorted(json.dumps(row, sort_keys=True, separators=(",", ":")) for row in rows)


def execute(capture, case, backend, extension, batch_size, restricted=True, max_candidates=5_000_000, only_path=None, ordered_pairs=False, analysis_limit=1000):
    findings, measurements = [], []
    for clause in capture["policy"]["clauses"]:
        if "relational" not in clause:
            continue
        plan = clause["relational"]
        schemas = case["schemas"][clause["id"]]
        inputs = case["rows"].get(clause["id"], {})
        paths = sorted({row[0] for rows in inputs.values() for row in rows}) if plan.get("partitionBy") == "file" else [None]
        for path in paths:
            if only_path is not None and path is not None and path != only_path:
                continue
            db = connect(extension, seconds=100, analysis_limit=analysis_limit)
            db.row_factory = sqlite_row
            started = time.process_time()
            try:
                for name, columns in schemas.items():
                    db.execute(f"CREATE TABLE {quote(name)} ({','.join(quote(column) for column in columns)})")
                    db.executemany(f"INSERT INTO {quote(name)} VALUES ({','.join('?' for _ in columns)})",
                                   [row for row in inputs.get(name, []) if (path is None or row[0] == path) and (only_path is None or row[0] == only_path)])
                for stage in plan.get("stages", []):
                    stage_started = time.process_time()
                    if stage["name"] == "descendants" and backend != "baseline":
                        db.row_factory = None
                        metrics = materialize(db, requested_roots(plan, restricted), backend, batch_size, max_candidates, ordered_pairs=ordered_pairs)
                        metrics.update({"path":path, "clause":clause["id"]})
                        db.row_factory = sqlite_row
                        measurements.append(metrics)
                        continue
                    query = resolve(stage["query"], case["options"])
                    if os.environ.get("RERAIL_REACH_TRACE"):
                        print(json.dumps({"clause":clause["id"], "stage":stage["name"], "start":True,
                            "plan":[row["detail"] for row in db.execute("EXPLAIN QUERY PLAN " + query)]}), file=sys.stderr, flush=True)
                    try:
                        db.execute(f"CREATE TEMP TABLE {quote(stage['name'])} AS {query}")
                    except Exception as error:
                        db.set_progress_handler(None,0)
                        for metric in measurements:
                            if metric["path"] == path and metric["clause"] == clause["id"] and metric.get("graph") is not None:
                                stats = db.execute("SELECT reach_stats(?) AS stats", (metric["graph"],)).fetchone()["stats"]
                                print(json.dumps({"failedStage":stage["name"], "rust":json.loads(stats)}), file=sys.stderr, flush=True)
                        raise RuntimeError(f"{clause['id']}/{stage['name']}: {error}") from error
                    for index, fields in enumerate(stage.get("indexes", [])):
                        db.execute(f"CREATE INDEX {quote(stage['name'] + '_' + str(index))} ON {quote(stage['name'])} ({','.join(quote(field) for field in fields)})")
                    if stage.get("indexes"):
                        db.execute(f"ANALYZE {quote(stage['name'])}")
                    if os.environ.get("RERAIL_REACH_TRACE"):
                        print(json.dumps({"clause":clause["id"], "stage":stage["name"], "cpuMs":(time.process_time()-stage_started)*1000}), file=sys.stderr, flush=True)
                findings.extend(db.execute(resolve(plan["query"], case["options"])).fetchall())
                for metrics in measurements:
                    if metrics["path"] == path and metrics["clause"] == clause["id"] and metrics["graph"] is not None:
                        db.row_factory = None
                        metrics["rustAfterConsumers"] = json.loads(db.execute("SELECT reach_stats(?)", (metrics["graph"],)).fetchone()[0])
                        db.row_factory = sqlite_row
            finally:
                db.close()
            measurements.append({"path":path, "clause":clause["id"], "pipelineCpuMs":(time.process_time()-started)*1000})
    # Preserve multiplicity: native SQL rejects duplicate diagnostic identities.
    # A duplicate row from an adapter must fail parity instead of disappearing.
    return findings, measurements


def sqlite_row(cursor, row):
    return dict(zip([column[0] for column in cursor.description], row))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True)
    parser.add_argument("--extension", required=True, help="Also supplies the engine's Rust regex semantics to every backend")
    backends = ["baseline", "sql", "scalar", "vtab", "vtab-direct", "interval", "interval-direct"]
    parser.add_argument("--backend", choices=[*backends, "all"], required=True)
    parser.add_argument("--batch-size", type=int, default=256)
    parser.add_argument("--all-roots", action="store_true")
    parser.add_argument("--ordered-pairs", action="store_true", help="Sort each SQL batch before inserting pairs")
    parser.add_argument("--analysis-limit", type=int, choices=[0,1000], default=1000, help="0 runs full ANALYZE; 1000 matches the engine's sampled statistics")
    parser.add_argument("--max-candidates", type=int, default=5_000_000)
    parser.add_argument("--offset", type=int, default=0)
    parser.add_argument("--limit", type=int, default=20)
    parser.add_argument("--output", required=True)
    parser.add_argument("--path", help="Replay one file from a production capture")
    parser.add_argument("--repeat", type=int, default=1)
    args = parser.parse_args()
    capture_path = Path(args.capture)
    capture = json.loads(capture_path.read_text())
    rows = []
    started = time.process_time()
    selected = capture["cases"][args.offset:args.offset + args.limit]
    for backend, iteration, case in itertools.product(backends if args.backend == "all" else [args.backend], range(args.repeat), selected):
        before = time.process_time()
        actual, metrics = execute(capture, case, backend, args.extension, args.batch_size,
                                  not args.all_roots, args.max_candidates, args.path, args.ordered_pairs, args.analysis_limit)
        expected = normalized([row for row in case["expected"] if args.path is None or row["path"] == args.path])
        observed = normalized(actual)
        rows.append({"name":case["name"], "backend":backend, "iteration":iteration, "positive":bool(expected), "expected":len(expected), "actual":len(observed),
                     "parity":expected == observed, "missing":sorted(set(expected)-set(observed)),
                     "extra":sorted(set(observed)-set(expected)), "cpuMs":(time.process_time()-before)*1000, "metrics":metrics})
    report = {"format":"rerail.reachability-replay/v1", "backend":args.backend,
              "orderedPairs":args.ordered_pairs,
              "analysisLimit":args.analysis_limit,
              "batchSize":args.batch_size, "restrictedRoots":not args.all_roots,
              "captureSha256":hashlib.sha256(capture_path.read_bytes()).hexdigest(),
              "nativeSha256":capture["nativeSha256"], "policySha256":capture["policySha256"],
              "extensionSha256":hashlib.sha256(Path(args.extension).read_bytes()).hexdigest() if args.extension else None,
              "commit":capture["commit"], "cpuMs":(time.process_time()-started)*1000,
              "sqliteVersion":sqlite3.sqlite_version, "platform":platform.platform(),
              "peakRssBytes":resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if sys.platform == "darwin" else 1024),
              "cases":len(rows), "positive":sum(row["positive"] for row in rows), "rows":rows,
              "note":"Captured-input SQL replay, not an end-to-end native engine benchmark. Parity compares complete SQL rows against native findings."}
    Path(args.output).write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key:value for key,value in report.items() if key != "rows"}))
    if any(not row["parity"] for row in rows):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
