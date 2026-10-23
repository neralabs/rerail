#!/usr/bin/env python3
"""One-off exactness probe for `required-contracts` (kept for the record). Its per-content stages
(`classes`, `members`, …) run on the file side and reach the repository only as anonymous
exports, so a parity run's stage outputs never show them. This writes probe copies of the original
and of the linked rewrite in which every such stage `X` gets a repository-side copy
`probe_X = SELECT x.*, n.probe_n FROM X x CROSS JOIN (SELECT count(*) AS probe_n FROM X) n` (a count
over all files cannot run per file), and every matcher input / per-content fact `I` a count `probe_I`; the findings query reads
each probe under a never-true filter so the split keeps it. Comparing the two runs' `probe_*`
outputs with `stages_equal.py` compares the per-content stages row for row on the snapshot.

usage: probe_contracts.py ORIGINAL_DOC LINKED_DOC OUT_DIR"""
import copy
import json
import sys
from pathlib import Path

STAGES = ["classes", "symbols", "members", "methods", "annotations", "returns", "parameters", "bases", "union_names"]


def probe(doc, out):
    doc = copy.deepcopy(doc)
    doc["clauses"] = [c for c in doc["clauses"] if c["id"] in ("shared", "required-contracts")]
    rel = next(c for c in doc["clauses"] if c["id"] == "required-contracts")["relational"]
    reads = []
    for s in STAGES:
        rel["stages"].append({"name": f"probe_{s}", "query": f"SELECT x.*,n.probe_n FROM {s} x CROSS JOIN (SELECT count(*) AS probe_n FROM {s}) n"})
        reads.append(f"probe_{s} WHERE probe_n<0")
    for i in rel["inputs"]:
        rel["stages"].append({"name": f"probe_{i}", "query": f"SELECT count(*) AS probe_n FROM {i}"})
        reads.append(f"probe_{i} WHERE probe_n<0")
    tail = "".join(f"\n UNION ALL SELECT '',0,0,'','','' FROM {r}" for r in reads)
    rel["query"] = {**rel["query"], "cases": {k: v + tail for k, v in rel["query"]["cases"].items()}}
    Path(out).write_text(json.dumps(doc, indent=1) + "\n")


if __name__ == "__main__":
    orig, linked, out = sys.argv[1:4]
    Path(out).mkdir(parents=True, exist_ok=True)
    probe(json.loads(Path(orig).read_text()), Path(out) / "probe-original.json")
    probe(json.loads(Path(linked).read_text()), Path(out) / "probe-rewrite.json")
