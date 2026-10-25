#!/usr/bin/env python3
"""Qualify native matcher fragments against complete-workload evidence.

Inputs come from fragment_integration_catalog_job and fragment_integration_workspace_job.
The output retains failures as evidence even when a ceiling rejects the run.
"""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess


def digest(value):
    return hashlib.sha256(value).hexdigest()


def key_digest(keys):
    return digest(json.dumps(sorted(keys), separators=(",", ":")).encode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("catalog", "native", "sql", "corpus", "binary", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    catalog, native, sql = [json.loads(p.read_bytes()) for p in (args.catalog, args.native, args.sql)]
    failures = []

    def require(condition, message):
        if not condition:
            failures.append(message)

    require(native["files"] == sql["files"] == 5725, "full 5,725-file workload required")
    require(native["sourceBytes"] == sql["sourceBytes"], "source sizes differ")
    require(native["policies"] == sql["policies"], "policy inventories differ")
    require(bool(native["keys"]), "workload must have positive findings")
    require(native["keys"] == sql["keys"], "cold finding sets differ")
    require(not native["failures"] and not sql["failures"], "cold coverage is incomplete")
    require(len(native["edits"]) == len(sql["edits"]) == 3, "three edits required")
    for i, (n, s) in enumerate(zip(native["edits"], sql["edits"])):
        require(n["keys"] == s["keys"], f"edit {i} finding sets differ")
        require(not n["failures"] and not s["failures"], f"edit {i} coverage is incomplete")
    for r in (native, sql):
        require(r["edits"][0]["keys"] != r["keys"], "first edit must add a finding")
        require(r["edits"][1]["keys"] != r["edits"][0]["keys"], "second edit must change findings")
        require(r["edits"][2]["keys"] == r["keys"], "last edit must restore findings")
    require(catalog["grammars"] == 146 and catalog["files"] == 597, "complete catalog required")
    for r in catalog["runs"]:
        require(r["compared"] == 87162, "every catalog rule/file pair must be compared")
        require(not r["unknownNative"] and not r["unknownSql"], "catalog coverage is incomplete")
        require(not r["mismatches"], "catalog raw captures differ")
    cpu_ratio = sql["cold"]["cpuNs"] / native["cold"]["cpuNs"]
    edit_ratio = statistics.median(e["time"]["cpuNs"] for e in sql["edits"]) / statistics.median(e["time"]["cpuNs"] for e in native["edits"])
    statements = sql["report"]["statements"]["count"]
    require(statements <= 100, "matching execution budget exceeded")
    require(cpu_ratio <= 1.30, "cold CPU ratio exceeds 1.30")
    require(edit_ratio <= 1.50, "median edit CPU ratio exceeds 1.50")
    require(sql["cold"]["peakRssBytes"] <= 1_610_612_736, "cold SQL peak RSS exceeds 1.5 GiB")
    profiles = sql["report"]["matcherFragments"]
    require(sum(len(p["statements"]) for p in profiles) == statements, "partition trace misses executions")
    require(all(not p["unknownFiles"] for p in profiles), "native boundaries report incomplete files")
    require(all(b["filters"] <= 1 for p in profiles for b in p["boundaries"]), "a native boundary runs more than once per partition")
    corpus_files = subprocess.check_output(["git", "-C", str(args.corpus), "ls-files", "-z"]).decode().split("\0")
    corpus_inventory = [(p, digest((args.corpus / p).read_bytes())) for p in corpus_files if p.endswith(".py")]
    source_paths = subprocess.check_output(["git", "diff", "--name-only", "HEAD", "--", "native/src"]).decode().splitlines()
    source_paths += subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard", "native/src"]).decode().splitlines()
    source_inventory = {p: digest(Path(p).read_bytes()) for p in sorted(set(source_paths)) if Path(p).is_file()}

    def engine_report(r):
        return {
            "compile": r["compile"], "cold": r["cold"],
            "findings": len(r["keys"]), "findingKeysSha256": key_digest(r["keys"]),
            "failures": r["failures"],
            "edits": [{"time": e["time"], "findings": len(e["keys"]), "findingKeysSha256": key_digest(e["keys"]), "failures": e["failures"]} for e in r["edits"]],
        }

    by_clause = {}
    names = {n["statement"]: f'{n["ruleId"]}:{n["clauseId"]}' for n in sql["report"]["compiledClauseNames"]}
    for profile in profiles:
        for i, boundary in enumerate(profile["boundaries"]):
            total = by_clause.setdefault(i, {"rule": names[i], "cpuNs": 0, "roots": 0, "rows": 0, "filters": 0})
            for target, source in (("cpuNs", "native_cpu_ns"), ("roots", "roots"), ("rows", "rows"), ("filters", "filters")):
                total[target] += boundary[source]
    matching_cpu = sum(s["cpuNs"] for p in profiles for s in p["statements"])
    boundary_cpu = sum(v["cpuNs"] for v in by_clause.values())

    result = {
        "accepted": not failures, "failures": failures,
        "ceilings": {"matchingExecutions": 100, "coldCpuRatio": 1.30, "medianEditCpuRatio": 1.50, "coldSqlPeakRssBytes": 1_610_612_736},
        "configuration": {"profile": "loop", "optLevel": 1, "allocator": "real jemalloc", "localTestLto": False, "features": ["unbounded-work"], "rustc": subprocess.check_output(["rustc", "--version"]).decode().strip()},
        "provenance": {"baseCommit": subprocess.check_output(["git", "rev-parse", "HEAD"]).decode().strip(), "corpusCommit": subprocess.check_output(["git", "-C", str(args.corpus), "rev-parse", "HEAD"]).decode().strip(), "corpusInventorySha256": digest(json.dumps(corpus_inventory, separators=(",", ":")).encode()), "changedRustSources": source_inventory, "binarySha256": digest(args.binary.read_bytes()), "rawReportsSha256": {n: digest(p.read_bytes()) for n, p in (("catalog", args.catalog), ("native", args.native), ("sql", args.sql))}},
        "workload": {"files": sql["files"], "sourceBytes": sql["sourceBytes"], "policies": sql["policies"], "policySource": "shipped Python catalog, not the unavailable original twelve policies"},
        "native": engine_report(native), "sql": engine_report(sql),
        "coldCpuRatio": cpu_ratio, "medianEditCpuRatio": edit_ratio,
        "matchingStatements": statements,
        "sqlCompilePhases": sql["report"]["compile"],
        "sqlSessions": sql["report"]["sessions"],
        "sqlProgram": sql["report"]["program"],
        "vmInstructions": sum(s["vm"] for p in profiles for s in p["statements"]),
        "matchingCpuNs": matching_cpu,
        "nativeBoundaryCpuNs": boundary_cpu,
        "sqlAndBoundaryOverheadCpuNs": matching_cpu - boundary_cpu,
        "largestNativeFragments": sorted(by_clause.values(), key=lambda v: v["cpuNs"], reverse=True)[:10],
        "nativeFilters": sum(b["filters"] for p in profiles for b in p["boundaries"]),
        "nativeRoots": sum(b["roots"] for p in profiles for b in p["boundaries"]),
        "peakRequestRows": max(b["peak_request_rows"] for p in profiles for b in p["boundaries"]),
        "compiler": {k: v for k, v in sql["report"]["relational"].items() if k != "registry"},
        "catalog": {"grammars": catalog["grammars"], "files": catalog["files"], "runs": [{k: v for k, v in r.items() if k != "detail"} for r in catalog["runs"]]},
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ("accepted", "failures", "coldCpuRatio", "medianEditCpuRatio", "matchingStatements", "vmInstructions")}, indent=2))
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
