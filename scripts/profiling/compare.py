#!/usr/bin/env python3
"""Compare matched baseline and experimental profile artifacts."""
import argparse
import json
from pathlib import Path


def compare(baseline, experiment, allow_policy_changes=False):
    before, after = baseline["parity"], experiment["parity"]
    if set(baseline.get("sampleModules", ["rerail_engine"])) != set(experiment.get("sampleModules", ["rerail_engine"])):
        raise ValueError("Profiler modes do not match: sampleModules")
    for key in ["trace", "external", "queryPlans"]:
        if baseline.get(key) != experiment.get(key):
            raise ValueError(f"Profiling modes do not match: {key}")
    if before.get("measuredPhases") != after.get("measuredPhases"):
        raise ValueError("Measured phases do not match")
    if not before.get("workloadSha256") or not after.get("workloadSha256"):
        raise ValueError("Profiles require a workload fingerprint from the current driver")
    for key in ["commit", "files", "bytes", "workloadSha256", "nativeSha256", "allocator"]:
        if before[key] != after[key]:
            raise ValueError(f"Profiles do not match: {key}")
    changed = {key: {"baseline": before["policyDigests"].get(key), "experiment": after["policyDigests"].get(key)}
               for key in before["policyDigests"].keys() | after["policyDigests"].keys()
               if before["policyDigests"].get(key) != after["policyDigests"].get(key)}
    if changed and (not allow_policy_changes or before["policyDigests"].keys() != after["policyDigests"].keys()):
        raise ValueError("Profiles do not match: policyDigests (SQL rewrites require --allow-policy-changes and identical rule sets)")
    for run in [baseline, experiment]:
        parity = run["parity"]
        if run["exitCode"] != 0 or run["timedOut"] or any(parity[key] for key in ["falsePositives", "falseNegatives", "failures", "incrementalDifferences"]):
            raise ValueError("Comparison requires completed runs with passing parity")
    phases = {}
    for name, key in [("cold", "coldMs"), ("unchanged", "unchangedFileMs"), ("edit", "editedFileMs")]:
        if before[key] is None or after[key] is None:
            continue
        cpu = lambda row: sum(row.get("phaseCpuMs", {}).get(name, {}).values()) or None
        phases[name] = {"baselineMs": before[key], "experimentMs": after[key],
                        "wallSpeedup": before[key] / after[key] if after[key] else None,
                        "baselineCpuMs": cpu(before), "experimentCpuMs": cpu(after)}
    return {"phases": phases, "baselinePeakRssBytes": before["peakRssBytes"], "experimentPeakRssBytes": after["peakRssBytes"],
            "changedSettings": {key: {"baseline": baseline.get(key), "experiment": experiment.get(key)}
                                for key in ["insertBatch", "nativeInput", "sharedInputIndex", "retainSqlStages"] if baseline.get(key) != experiment.get(key)},
            "changedPolicies": changed,
            "nativeSha256": before["nativeSha256"], "warning": "Matched single runs on a shared host; use repeated CPU and wall measurements."}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("experiment", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--allow-policy-changes", action="store_true", help="Compare authored SQL rewrites; report every changed policy digest")
    args = parser.parse_args()
    result = compare(json.loads(args.baseline.read_text()), json.loads(args.experiment.read_text()), args.allow_policy_changes)
    text = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(text)
    print(text, end="")
