#!/usr/bin/env python3
"""Merges the clause cover (`cover.py` → cover.json) and the construct picks (constructs.json) into the
distilled loop subset: `$RERAIL_BENCH_HOME/distill/subset.json` and `subset.txt` (one relative path per line).

A file is in the subset when the cover chose it (a hit or clean file for some clause) or the construct map
chose it (a hit or clean file for some hard construct). Each file keeps every reason it was chosen.
`loop.ts select --from <subset.json>` turns the result into benchmark/data-layer/loop-subset.json.
"""
import json
import os

home = os.environ.get("RERAIL_BENCH_HOME", os.path.expanduser("~/rerail-bench"))
distill = os.path.join(home, "distill")
cover = json.load(open(os.path.join(distill, "cover.json")))
constructs = json.load(open(os.path.join(distill, "constructs.json")))

files = {}
for entry in cover["files"]:
    files[entry["path"]] = {
        "bytes": entry["bytes"],
        "native_ms": entry.get("native_ms"),
        "why": ["cover:" + "/".join(map(str, why)) for why in entry["why"]],
    }
for name, construct in constructs["constructs"].items():
    for side in ("hit", "clean"):
        for pick in construct.get(side, []):
            path = pick["path"] if isinstance(pick, dict) else pick
            files.setdefault(path, {"bytes": None, "native_ms": None, "why": []})["why"].append(
                f"construct:{name}:{side}"
            )
for path, meta in constructs["files"].items():
    if path in files:
        entry = files[path]
        entry["bytes"] = entry["bytes"] or meta["bytes"]
        if entry["native_ms"] is None:
            entry["native_ms"] = meta["allMs"]

subset = {
    "format": "rerail.sql-loop-subset/v2",
    "sources": ["cover.json", "constructs.json"],
    "oracle": "native run on this subset (not the full-tree oracle): calls/select_when/budget clauses are cross-file",
    "files": [{"path": path, **meta} for path, meta in sorted(files.items())],
}
subset["totals"] = {
    "files": len(files),
    "bytes": sum(f["bytes"] or 0 for f in subset["files"]),
    "native_ms_single_thread": round(sum(f["native_ms"] or 0 for f in subset["files"]), 1),
    "missing_cost": [f["path"] for f in subset["files"] if f["native_ms"] is None],
}
json.dump(subset, open(os.path.join(distill, "subset.json"), "w"), indent=1)
open(os.path.join(distill, "subset.txt"), "w").write("\n".join(sorted(files)) + "\n")
print(json.dumps(subset["totals"]))
