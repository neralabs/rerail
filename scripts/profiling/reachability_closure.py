#!/usr/bin/env python3
"""Compare every requested pair on a captured real graph with original SQL closure."""
import argparse
import hashlib
import json
from pathlib import Path
import time

from reachability_probes import prefix
from reachability_replay import resolve
from reachability_sql import materialize, requested_roots


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True)
    parser.add_argument("--extension", required=True)
    parser.add_argument("--path", required=True)
    parser.add_argument("--backend", choices=["sql", "vtab", "vtab-direct", "interval", "interval-direct", "scalar"], required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    capture = json.loads(Path(args.capture).read_text())
    case = capture["cases"][0]
    # The checker constructs two closures and compares both set differences.
    # Its work allowance covers three workloads; the controller still caps time.
    db, plan = prefix(capture, case, args.path, args.extension, work=600_000_000)
    query = next(stage["query"] for stage in plan["stages"] if stage["name"] == "descendants")
    started = time.process_time()
    db.execute("CREATE TEMP TABLE truth_descendants AS " + resolve(query, case["options"]))
    db.execute("CREATE INDEX truth_root ON truth_descendants(root,node)")
    truth_cpu_ms = (time.process_time()-started)*1000
    metrics = materialize(db, requested_roots(plan), args.backend)
    truth = "SELECT __path,root,node FROM truth_descendants WHERE root IN (SELECT k.key FROM reach_requests r JOIN reach_keys k ON k.compact=r.root)"
    actual = "SELECT __path,root,node FROM descendants"
    missing = db.execute(truth + " EXCEPT " + actual + " LIMIT 5").fetchall()
    extra = db.execute(actual + " EXCEPT " + truth + " LIMIT 5").fetchall()
    expected_count = db.execute("SELECT count(*) FROM (" + truth + ")").fetchone()[0]
    actual_count = db.execute("SELECT count(*) FROM descendants").fetchone()[0]
    all_count = db.execute("SELECT count(*) FROM truth_descendants").fetchone()[0]
    report = {"format":"rerail.reachability-closure/v1", "backend":args.backend, "path":args.path,
              "allPairs":all_count, "requestedPairs":expected_count, "actualPairs":actual_count,
              "missing":missing, "extra":extra, "parity":not missing and not extra and expected_count == actual_count,
              "sqlWorkBudget":600_000_000,
              "metrics":metrics, "truthCpuMs":truth_cpu_ms,
              "captureSha256":hashlib.sha256(Path(args.capture).read_bytes()).hexdigest(),
              "note":"Full pair-set comparison. Timing includes a retained truth table and is not used as a performance benchmark."}
    Path(args.output).write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    db.close()
    if not report["parity"]: raise SystemExit(1)


if __name__ == "__main__":
    main()
