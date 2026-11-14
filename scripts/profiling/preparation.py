"""Rank equivalent candidate relations without confusing logical reuse with file overlap."""
from __future__ import annotations

import argparse
import collections
import json
from pathlib import Path


def summarize(directory: Path) -> dict:
    profile = json.loads((directory / "profile.json").read_text())
    parity = json.loads((directory / "parity.json").read_text())
    events = [json.loads(line) for line in (directory / "trace.jsonl").read_text().splitlines()]
    batches = [event for event in events if event["phase"] == "anchor-batch"]
    constructions = collections.defaultdict(list)
    census = {}
    for event in events:
        if event["phase"] != "anchor-relation-census":
            continue
        for relation in (event.get("detail") or {}).get("relations", []):
            census[(event["thread"], relation["name"], event["startUnixUs"])] = relation

    totals = collections.Counter()
    for index, batch in enumerate(batches):
        start = batch["startUnixUs"]
        end = start + batch["wallNs"] / 1000
        rows = {
            name: relation["rows"]
            for (thread, name, time), relation in census.items()
            if thread == batch["thread"] and start <= time <= end
        }
        for event in events:
            if event["thread"] != batch["thread"] or not start <= event["startUnixUs"] <= end:
                continue
            phase = event["phase"]
            if phase in ("anchor-materialize", "anchor-index", "anchor-results", "anchor-prepare", "anchor-release"):
                totals[phase] += (event.get("cpuNs") or 0) / 1e6
            detail = event.get("detail") or {}
            identity = detail.get("relationId")
            if phase != "anchor-materialize" or identity is None:
                continue
            constructions[identity].append({
                "batch": index,
                "files": (batch.get("detail") or {}).get("files"),
                "name": event["name"],
                "class": detail.get("dependencyClass"),
                "cpuMs": (event.get("cpuNs") or 0) / 1e6,
                "rows": rows.get(event["name"]),
                "vmSteps": detail.get("vmSteps"),
            })

    duplicates = []
    for identity, builds in constructions.items():
        by_batch = collections.defaultdict(list)
        for build in builds:
            by_batch[build["batch"]].append(build)
        repeated = [build for group in by_batch.values() for build in group[1:]]
        if len(builds) > 1:
            duplicates.append({"relationId": identity, "builds": builds,
                               "withinBatchRepeatedCpuMs": sum(build["cpuMs"] for build in repeated)})
    duplicates.sort(key=lambda row: row["withinBatchRepeatedCpuMs"], reverse=True)
    return {
        "format": "rerail.sql-preparation-diagnostic/v1",
        "directory": str(directory), "nativeSha256": parity["nativeSha256"],
        "workloadSha256": parity["workloadSha256"], "policyDigests": parity["policyDigests"],
        "parityPassed": not any(parity[key] for key in ("falsePositives", "falseNegatives", "failures")),
        "complete": profile["exitCode"] == 0 and not profile["timedOut"],
        "batchFiles": [(batch.get("detail") or {}).get("files") for batch in batches],
        "exclusiveCpuMs": dict(totals), "logicalConstructions": sum(map(len, constructions.values())),
        "distinctConstructedRelations": len(constructions),
        "withinBatchRepeatedCpuMs": sum(row["withinBatchRepeatedCpuMs"] for row in duplicates),
        "duplicates": duplicates,
        "limitations": [
            "Repeated CPU estimates exclude retained-access and maintenance costs.",
            "Cross-batch equality does not prove equal file domains. Older captures lack file identities.",
            "Timings include diagnostic instrumentation and are not untraced performance results.",
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = summarize(args.directory)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "duplicates"}, indent=2))
    if not result["parityPassed"] or not result["complete"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
