#!/usr/bin/env python3
"""Summarize the full integration trace and the focused batch diagnostic separately."""
import argparse
import collections
import gzip
import hashlib
import json
import pathlib
import statistics


def read(path):
    with gzip.open(path, "rt") if str(path).endswith(".gz") else open(path) as f:
        return json.load(f)


def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def median_pair(runs):
    values = {
        str(shared): int(statistics.median(r["cpuNs"] for r in runs if r["shared"] == shared))
        for shared in (False, True)
    }
    return {
        "baselineMedianCpuNs": values["False"],
        "sharedMedianCpuNs": values["True"],
        "sharedOverBaseline": values["True"] / values["False"],
    }


def total(profile, key):
    return sum(statement[key] for statement in profile["statements"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--full", required=True)
    parser.add_argument("--audit", required=True)
    parser.add_argument("--catalog", default="benchmark/sql-matcher-fragment-integration-catalog.json.gz")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    whole = read(args.full)
    report = whole["report"]
    audit = read(args.audit)
    catalog = read(args.catalog)
    metrics = [collections.Counter() for _ in report["compiledClauseNames"]]
    for batch in report["matcherFragments"]:
        for aggregate, boundary in zip(metrics, batch["boundaries"], strict=True):
            aggregate.update({key: boundary[key] for key in (
                "roots", "rows", "filters", "native_cpu_ns", "logical_unknown_roots"
            )})
    clauses = [{**name, **metric} for name, metric in zip(report["compiledClauseNames"], metrics, strict=True)]
    sql_arms = [arm for query in catalog["runs"][0]["detail"]["sql"] for arm in query.split(" UNION ALL ")]
    shapes = collections.Counter(
        "mixed" if "demand=" in arm else "native" if "FROM matcher_" in arm else "sql"
        for arm in sql_arms
    )
    isolated = []
    for arm in audit["arms"]:
        entry = {key: arm[key] for key in ("id", "rule", "kindPredicateImpliedByAnchors")}
        entry["baselineVm"] = total(arm["baseline"], "vm")
        entry["baselineCpuNs"] = total(arm["baseline"], "cpuNs")
        if "withoutRedundantFilter" in arm:
            entry["withoutRedundantFilterVm"] = total(arm["withoutRedundantFilter"], "vm")
            entry["withoutRedundantFilterCpuNs"] = total(arm["withoutRedundantFilter"], "cpuNs")
        isolated.append(entry)
    shared = audit["async"]
    async_ids = set(shared["rules"])
    native_roots = lambda p: sum(b["roots"] for b in p["boundaries"])
    baseline = next(r for r in shared["runs"] if not r["shared"])
    factored = next(r for r in shared["runs"] if r["shared"])
    output = {
        "inputs": {key: {"path": str(path), "sha256": digest(path)} for key, path in (
            ("full", args.full), ("audit", args.audit), ("catalog", args.catalog)
        )},
        "wholeJobExistingMeasurement": {
            "files": whole["files"], "cold": whole["cold"],
            "matchingStatements": sum(len(p["statements"]) for p in report["matcherFragments"]),
            "matchingCpuNs": sum(total(p, "cpuNs") for p in report["matcherFragments"]),
            "vm": sum(total(p, "vm") for p in report["matcherFragments"]),
            "native": sum(metrics, collections.Counter()), "shapes": shapes,
            "statistics": report["relational"]["statistics"],
            "clausesByNativeCpu": sorted(clauses, key=lambda c: c["native_cpu_ns"], reverse=True),
            "sixAsyncNativeCpuNs": sum(c["native_cpu_ns"] for c in clauses if c["ruleId"] in async_ids),
            "sixAsyncRoots": sum(c["roots"] for c in clauses if c["ruleId"] in async_ids),
        },
        "catalogExperiment": {
            "files": audit["files"], "rules": audit["rules"],
            "scope": audit["scope"], "memo": median_pair(audit["memo"]),
            "timingScope": {
                "memo": "Native file-first runner in both variants; includes parsing; only memo lifetime changes.",
                "async": "Includes producer, consumer-demand construction, and consumers; excludes preparation and common input-domain construction.",
                "sqlArms": "One execution per variant, with isolated output widths; CPU is a single sample, VM counts are deterministic.",
                "wholeJobImpact": "Not measured for these diagnostic alternatives.",
            },
            "isolatedArmsByVm": sorted(isolated, key=lambda a: a["baselineVm"], reverse=True),
            "async": {**median_pair(shared["runs"]),
                "rules": shared["rules"], "inputRowsPerRule": shared["inputRowsPerRule"],
                "identicalDomainPairs": shared["identicalDomainPairs"], "unionRows": shared["unionRows"],
                "admitted": factored["detail"]["admitted"],
                "baselineRoots": native_roots(baseline["detail"]),
                "producerRoots": native_roots(factored["detail"]["producer"]),
                "residualRoots": native_roots(factored["detail"]["consumers"]),
            },
        },
    }
    pathlib.Path(args.output).write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
