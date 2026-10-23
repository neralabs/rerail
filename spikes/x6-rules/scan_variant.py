#!/usr/bin/env python3
"""Test variants for state-flow-project-1 (exactness evidence only). Under the suite's
configuration (`entryPoint` = `all`) the rule's gate holds nowhere (`filePath: \\b\\B`), so every
stage is empty on every snapshot. The rule is live under `entryPoint` = `scan_project_sources`
(the native analyzer's project scan). This writes the original clause and the linked rewrite with
that option's default switched, each alone in its document, for the same pipeline and harness:

- `linked/scan/original/data-layer-serverplan-state-tripwires.json` (shared + the original clause)
- `linked/scan/rewrite/data-layer-serverplan-state-tripwires.json` (`linked/sfp.json`, switched)
- `linked/plain/{original,rewrite}/...`: the same two, default configuration (facts-a harness)
- quality-conditional-1 (live under the default): `linked/plain/{original,rewrite}/data-layer-serverplan-quality.json`."""
import copy
import json
from pathlib import Path

HERE = Path(__file__).parent


def only(doc, cid):
    d = copy.deepcopy(doc)
    d["clauses"] = [c for c in d["clauses"] if c["id"] in ("shared", cid)]
    return d


def scan(doc):
    d = copy.deepcopy(doc)
    d["clauses"][0]["define"]["options"]["entryPoint"]["default"] = "scan_project_sources"
    return d


ST = json.loads((HERE / "original/data-layer-serverplan-state-tripwires.json").read_text())
QU = json.loads((HERE / "original/data-layer-serverplan-quality.json").read_text())
SFP = json.loads((HERE / "linked/sfp.json").read_text())
QC = json.loads((HERE / "linked/qc.json").read_text())
out = {
    "scan/original/data-layer-serverplan-state-tripwires.json": scan(only(ST, "state-flow-project-1")),
    "scan/rewrite/data-layer-serverplan-state-tripwires.json": scan(SFP),
    "plain/original/data-layer-serverplan-state-tripwires.json": only(ST, "state-flow-project-1"),
    "plain/rewrite/data-layer-serverplan-state-tripwires.json": SFP,
    "plain/original/data-layer-serverplan-quality.json": only(QU, "quality-conditional-1"),
    "plain/rewrite/data-layer-serverplan-quality.json": QC,
}
for name, d in out.items():
    p = HERE / "linked" / name
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(d, indent=1) + "\n")
