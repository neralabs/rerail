#!/usr/bin/env python3
"""P3a memory attribution of one greplay run directory (S3 § 15.2/15.3's method).

Arranged records from results.json → arrangements (differential's logging, per named
arrangement), split by role (`n<node>:<role>` with the trailing index dropped) and, when the run
had FACTS_PROFILE=1, by plan and by the linker (`views/*` stages) through profile.json → stages.
Prints JSON: totals, per role, per plan, the linker split, RSS samples, interner, exported rows.

usage: p3a_attrib.py RUN_DIR [RUN_DIR...]
"""
import collections
import json
import re
import sys
from pathlib import Path


def attrib(run: Path) -> dict:
    d = json.loads((run / "results.json").read_text())
    roles: collections.Counter = collections.Counter()
    for name, recs, _bytes in d["arrangements"]["by_operator"]:
        role = re.sub(r"\d+", "", re.sub(r"^(n\d+|fix\d+):", "", name))
        roles[role] += recs
    out = {
        "run": str(run),
        "arranged_total": d["arrangements"]["total_records"],
        "by_role": dict(roles.most_common()),
        "rss": [{k: s.get(k) for k in ("at", "rss_mb", "footprint_mb", "arranged")} for s in d["rss"]],
        "rss_loaded_mb": d["load"].get("rss_loaded_mb"),
        "resident": d.get("resident"),
        "latency_single_file": d.get("latency", {}).get("single_file_all"),
        "exactness": {k: d["exactness"].get(k) for k in ("checks", "worktree_checks", "differences", "scratch_dataflows")},
        "runtime_refusals_live": len(d.get("runtime_refusals_live") or []),
    }
    prof = run / "profile.json"
    if prof.exists():
        p = json.loads(prof.read_text())
        per_plan: collections.Counter = collections.Counter()
        linker = 0
        for s in p["stages"]:
            plan = s["stage"].split(":")[0]
            per_plan[plan] += s["arranged"]
            if "/views/" in s["stage"] or ":views/" in s["stage"]:
                linker += s["arranged"]
        out["by_plan"] = dict(per_plan.most_common())
        out["linker_views"] = linker
        out["profiled_total"] = sum(per_plan.values())
    return out


if __name__ == "__main__":
    print(json.dumps([attrib(Path(a)) for a in sys.argv[1:]], indent=1))
