#!/usr/bin/env python3
"""Gate O125 step 07 profiles against a baseline from the same replay.

Run spike_o125_replay with RERAIL_PROFILE for each binary first. Keep run.log and
O125_SAVE's rows/07.txt beside each profile directory. The replay prints parity;
this gate requires it and compares the saved outputs. It checks recorded native work,
not total CPU: whole-plan
execution omits step counts, and counts can span more than one planned order.
"""
import argparse
import csv
import json
import re
from pathlib import Path


def measure(path):
    with path.open() as stream:
        rows = list(csv.DictReader(stream, delimiter="\t"))
    insertions = [r for r in rows if r["plan"].startswith("join ifn__ins(")
                  and r["plan"].split(": ", 1)[1].startswith("fv__ins(")]
    return {
        "total": sum(int(r["work"]) for r in rows),
        "ifn": sum(int(r["work"]) for r in rows if r["plan"].startswith("join ifn")),
        "retval": sum(int(r["work"]) for r in rows if r["plan"].startswith("seed retval(")),
        "ifnFvInsertWork": sum(int(r["work"]) for r in insertions),
        "ifnFvInsertPlans": insertions,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    args = parser.parse_args()
    result, failures = {}, []
    for label, path in [("baseline", args.baseline), ("candidate", args.candidate)]:
        log = (path.parent / "run.log").read_text()
        if not re.search(r"o125r 07 .*same as full: true", log) or "same as full: false" in log:
            failures.append(f"{label} lacks successful incremental/full output parity")
    baseline_rows = (args.baseline.parent / "rows/07.txt").read_bytes()
    candidate_rows = (args.candidate.parent / "rows/07.txt").read_bytes()
    if not baseline_rows or candidate_rows != baseline_rows:
        failures.append("saved outputs differ from the nonempty baseline")
    for phase in ["06-cold", "07-session", "07-full"]:
        before = measure(args.baseline / f"native-{phase}.tsv")
        after = measure(args.candidate / f"native-{phase}.tsv")
        result[phase] = {"before": before, "after": after}
        for label, counts in [("baseline", before), ("candidate", after)]:
            if not counts["total"] or not counts["retval"]:
                failures.append(f"{phase}: {label} lacks the expected work counters")
        for key in ["total", "retval"]:
            if after[key] > before[key]:
                failures.append(f"{phase}: {key} work regressed")
        if phase == "07-session":
            if len(after["ifnFvInsertPlans"]) != 2:
                failures.append("expected both source and annotation insertion probes")
            if after["ifnFvInsertWork"] > 2000:
                failures.append("targeted insertion probes exceed 2000 work units")
    print(json.dumps({"measurements": result, "failures": failures}, indent=2))
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
