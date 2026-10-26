#!/usr/bin/env python3
"""Y2's report numbers from one `facts-b y2` run: per-edit cost (scoped / unscoped) per rule,
the work counts behind it, the input churn per edit by table, and the maintained state across
worktrees (full copies vs base + delta). usage: y2_summary.py Y2_OUT_DIR"""
import json
import sys
from pathlib import Path


def pct(v, p):
    v = sorted(v)
    return v[round((len(v) - 1) * p)] if v else None


def dist(v):
    return {"p50": pct(v, 0.5), "p95": pct(v, 0.95), "p99": pct(v, 0.99), "max": max(v) if v else None, "n": len(v)}


def main():
    out = Path(sys.argv[1])
    res = json.loads((out / "results.json").read_text())
    ev = json.loads((out / "events.json").read_text())
    edits = [e for e in ev if e["group"] in ("edit", "new_worktree_edit")]
    s = {"checks": res["checks"], "differences": res["differences"], "edits": len(edits), "events": len(ev)}
    s["ms"] = {k: dist([e[k] for e in edits if e.get(k) is not None]) for k in ("dc_scoped_ms", "dc_unscoped_ms", "ef_scoped_ms", "ef_unscoped_ms", "facts_ms")}
    s["dc_counts"] = {k: dist([e["dc"][k] for e in edits]) for k in ("changes", "dirty_states", "dirty_reached", "edges_deleted", "edges_inserted", "over_deleted", "rederived", "propagated", "toggled", "findings_evaluated", "findings_changed", "succ_calls", "pred_checks")}
    s["ef_counts"] = {k: dist([e["ef"][k] for e in edits]) for k in ("sources", "surfaces_recomputed", "groups", "families", "findings_changed")}
    tabs = sorted({t for e in edits for t in e["changes_by_table"]})
    s["changes_by_table"] = {t: dist([e["changes_by_table"].get(t, 0) for e in edits]) for t in tabs}
    s["edits_changing_findings"] = {"dc": sum(1 for e in edits if e["dc"]["findings_changed"]), "ef": sum(1 for e in edits if e["ef"]["findings_changed"])}
    loads = res["loads"]
    full = sum(l["dc"]["reached_flow"] + l["dc"]["reached_annotated"] + l["dc"]["reached_dispatch"] for l in loads)
    first = loads[0]["dc"]["reached_flow"] + loads[0]["dc"]["reached_annotated"] + loads[0]["dc"]["reached_dispatch"]
    delta = sum(l["reached_vs_first"]["only_here"] + l["reached_vs_first"]["only_first"] for l in loads[1:])
    s["reached_state"] = {"worktrees": len(loads), "states_full_copies": full, "states_base_plus_delta": first + delta, "per_worktree_range": [min(l["dc"]["reached_flow"] + l["dc"]["reached_annotated"] + l["dc"]["reached_dispatch"] for l in loads), max(l["dc"]["reached_flow"] + l["dc"]["reached_annotated"] + l["dc"]["reached_dispatch"] for l in loads)],
                          "dispatch_states_range": [min(l["dc"]["reached_dispatch"] for l in loads), max(l["dc"]["reached_dispatch"] for l in loads)], "dispatch_by_class_range": [min(l["dispatch_by_class"] for l in loads), max(l["dispatch_by_class"] for l in loads)],
                          "bytes_per_state": 12}
    s["scratch_at_load_ms"] = {"dc": dist([l["dc_scratch_ms"] for l in loads]), "ef": dist([l["ef_scratch_ms"] for l in loads])}
    s["ef_state_words"] = dist([l["ef"]["state_words"] for l in loads])
    print(json.dumps(s, indent=1))


if __name__ == "__main__":
    main()
