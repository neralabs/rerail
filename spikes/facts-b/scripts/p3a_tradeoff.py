#!/usr/bin/env python3
"""P3a: the memory/latency trade-off per stage, M1 against M2.

Joins a greplay FACTS_PROFILE run (arranged records per stage: what full maintenance holds) with an
hreplay run (exclusive batch seconds and rows out per stage, summed over its from-scratch runs:
what recomputing costs), by `plan:clause/stage`. For each plan it prints the totals and the cost
of three choices of the maintained set S (S downward closed; every stage outside S recomputed per
affected edit):

  M2   S = {}                               resident: exports + membership + results
  M2L  S = the plan's membership-only cone  the stages over membership leaves only (`views/*`:
                                           module_of and the linker), maintained; the rest recomputed
  M1   S = every stage                      resident: every arrangement

Stage names are used here for the report only (diagnostics, INVARIANTS I3).

usage: p3a_tradeoff.py GREPLAY_PROFILE_RUN HREPLAY_RUN [--bytes-per-record B]
"""
import argparse
import collections
import json
from pathlib import Path

ap = argparse.ArgumentParser()
ap.add_argument("greplay")
ap.add_argument("hreplay")
ap.add_argument("--bytes-per-record", type=float, default=None, help="default: the greplay run's (RSS - interner - exported rows) / records")
a = ap.parse_args()

g = json.loads((Path(a.greplay) / "profile.json").read_text())
gr = json.loads((Path(a.greplay) / "results.json").read_text())
h = json.loads((Path(a.hreplay) / "results.json").read_text())

arranged = collections.Counter()
for s in g["stages"]:
    arranged[s["stage"]] += s["arranged"]
total_records = sum(arranged.values())
if a.bytes_per_record is None:
    rss = gr["rss"][-1]["rss_mb"] * 2**20
    res = gr.get("resident", {})
    bpr = (rss - res.get("interned_bytes", 0) - res.get("exported_row_bytes", 0)) / max(total_records, 1)
else:
    bpr = a.bytes_per_record

out = {"bytes_per_record": bpr, "greplay": a.greplay, "hreplay": a.hreplay, "plans": []}
# Per plan: (stage → (seconds per run, rows per run)), runs, p50 ms. An hreplay run either ran
# plans one at a time (`plans[].top_stages_secs_rows`) or a graph's plans together (`runs[]`, a
# stage labelled by the clause that first built it, counted once per run that read it).
per_plan = {}
if "runs" in h:
    for r in h["runs"]:
        n = max(r["ms"]["n"], 1)
        for s, t, rows in r["stages_secs_rows"]:
            plan = s.split("/")[0]
            per_plan.setdefault(plan, {"stages": {}, "p50": r["ms"]["p50_ms"]})
            st = per_plan[plan]["stages"].setdefault(f"{plan}:{s}", [0.0, 0.0])
            st[0] += t / n
            st[1] += rows / n
else:
    for p in h["plans"]:
        runs = max(p["runs"], 1)
        per_plan[p["plan"]] = {"stages": {f"{p['plan']}:{s}": [t / runs, r / runs] for s, t, r in p["top_stages_secs_rows"]}, "p50": p["ms"]["p50_ms"]}
for name, pp in per_plan.items():
    stages = pp["stages"]
    keys = set(stages) | {k for k in arranged if k.split(":")[0] == name}
    rows = []
    for k in sorted(keys):
        secs, rws = stages.get(k, (0.0, 0))
        rows.append({"stage": k.split(":", 1)[1], "arranged": arranged.get(k, 0), "batch_ms_per_run": secs * 1e3, "rows_per_run": rws, "membership_cone": "/views/" in k})
    tot_arr = sum(r["arranged"] for r in rows)
    tot_ms = sum(r["batch_ms_per_run"] for r in rows)
    cone_arr = sum(r["arranged"] for r in rows if r["membership_cone"])
    cone_ms = sum(r["batch_ms_per_run"] for r in rows if r["membership_cone"])
    out["plans"].append({
        "plan": name,
        "batch_ms_per_run_p50": pp["p50"],
        "arranged": tot_arr,
        "M2": {"maintained_records": 0, "recompute_ms": tot_ms},
        "M2L": {"maintained_records": cone_arr, "maintained_mb": cone_arr * bpr / 2**20, "recompute_ms": tot_ms - cone_ms},
        "M1": {"maintained_records": tot_arr, "maintained_mb": tot_arr * bpr / 2**20, "recompute_ms": 0.0},
        "stages_by_cost": sorted(rows, key=lambda r: -r["batch_ms_per_run"])[:25],
    })
print(json.dumps(out, indent=1))
