#!/usr/bin/env python3
"""E3 summary: per-edit latency and counts, memory against divergence, exactness.

    e3_summary.py RUN_DIR [--json OUT]

Reads RUN_DIR/events.json and RUN_DIR/results.json written by `facts-b lattice maint`.
Percentiles as Y3's (`y3/replay.rs::pct`): sorted, index round((n - 1) p).
"""
import json
import sys
from pathlib import Path


def pct(v, p):
    if not v:
        return 0.0
    s = sorted(v)
    return s[int(round((len(s) - 1) * p))]


def summ(v):
    return {"n": len(v), "p50": pct(v, 0.5), "p95": pct(v, 0.95), "p99": pct(v, 0.99), "max": pct(v, 1.0), "mean": sum(v) / max(len(v), 1)}


def fmt(s, nd=3):
    return " / ".join(f"{s[k]:.{nd}f}" if isinstance(s[k], float) else f"{s[k]:,}" for k in ("p50", "p95", "p99", "max"))


SINGLE = ("edit", "new_worktree_edit")


def latency(evs):
    """Per population: lattice CSR (Y2's scope + rows), values, sum, set; Y2 dead code."""
    out = {}
    pops = {
        "single-file (all)": [e for e in evs if e["group"] in SINGLE],
        "single-file, 300-event prefix (Y2's 298)": [e for e in evs if e["group"] in SINGLE and e["i"] < 300],
        "bulk": [e for e in evs if e["group"] == "bulk"],
        "commit": [e for e in evs if e["group"] in ("commit", "commit_init")],
    }
    for name, es in pops.items():
        es = [e for e in es if e["y2"] is not None]
        scope = [e["y2"]["scope_ms"] for e in es]
        rows = [e["lat"]["rows_ms"] for e in es]
        csr = [a + b for a, b in zip(scope, rows)]
        lvl = [e["lat"]["lvl_ms"] for e in es]
        st = [e["lat"]["set_ms"] for e in es]
        bf = [e["lat"].get("set_bf_ms", 0.0) for e in es]
        tot = [a + b for a, b in zip(csr, lvl)]
        tot_bf = [a + b for a, b in zip(csr, bf)]
        y2 = [e["y2"]["dc_ms"] for e in es]
        y2nf = [e["y2"]["dc_ms"] - e["y2"]["findings_ms"] for e in es]
        out[name] = {
            "lattice_csr_ms (y2 scope + rows)": summ(csr), "of which y2 scope_ms": summ(scope), "of which rows_ms": summ(rows),
            "rows: succ_ms": summ([e["lat"]["rows"].get("succ_ms", 0.0) for e in es]), "rows: map_ms": summ([e["lat"]["rows"].get("map_ms", 0.0) for e in es]),
            "rows: layer_ms": summ([e["lat"]["rows"].get("layer_ms", 0.0) for e in es]),
            "lattice_values_ms (min_level)": summ(lvl), "lattice_sum_ms (csr + min_level)": summ(tot),
            "set_ms (DRed)": summ(st), "set_bf_ms (B/F)": summ(bf), "lattice_sum_set_bf_ms (csr + set B/F)": summ(tot_bf),
            "y2_dc_ms": summ(y2), "y2_dc_minus_findings_ms": summ(y2nf),
            "load_1m": summ([e["load_1m"] for e in es]),
        }
    return out


def counts(evs):
    es = [e for e in evs if e["group"] in SINGLE and e["y2"] is not None]
    g = lambda f: summ([f(e) for e in es])
    L = lambda k: g(lambda e: e["lat"]["rows"][k])
    out = {
        "rows: dirty_states (Y2's scope)": L("dirty_states"),
        "rows: dirty_nodes": L("dirty_nodes"),
        "rows: recomputed": L("recomputed"),
        "rows: recomputed flow": g(lambda e: e["lat"]["rows"]["recomputed_by_kind"][0]),
        "rows: recomputed annotated": g(lambda e: e["lat"]["rows"]["recomputed_by_kind"][1]),
        "rows: recomputed dispatch": g(lambda e: e["lat"]["rows"]["recomputed_by_kind"][2]),
        "rows: succ targets": L("succ_targets"),
        "rows: changed": L("rows_changed"),
        "rows: changed dispatch": g(lambda e: e["lat"]["rows"]["rows_changed_by_kind"][2]),
        "rows: ids added": L("ids_added"),
        "edges deleted": L("edges_del"),
        "edges inserted": L("edges_ins"),
        "seeds deleted + inserted": g(lambda e: e["lat"]["rows"]["seeds_del"] + e["lat"]["rows"]["seeds_ins"]),
        "transposed rows changed": L("rev_rows_changed"),
        "fwd layer bits set + cleared": g(lambda e: e["lat"]["rows"]["fwd"]["bits_set"] + e["lat"]["rows"]["fwd"]["bits_cleared"]),
        "fwd layer slots shifted": g(lambda e: e["lat"]["rows"]["fwd"]["slots_shifted"]),
        "fwd layer blocks moved": g(lambda e: e["lat"]["rows"]["fwd"]["blocks_moved"]),
        "fwd + rev layer compactions": g(lambda e: e["lat"]["rows"]["fwd"]["compactions"] + e["lat"]["rows"]["rev"]["compactions"]),
    }
    for k in ("checked", "invalidated", "inval_rounds", "support_edges", "inval_edges", "rederive_edges", "rederived", "prop_rounds", "prop_frontier", "prop_edges", "changed", "toggled"):
        out[f"min_level: {k}"] = g(lambda e, k=k: e["lat"]["lvl"][k])
    for k in ("over_deleted", "over_rounds", "over_edges", "rederive_edges", "rederived", "prop_rounds", "prop_edges", "toggled"):
        out[f"set: {k}"] = g(lambda e, k=k: e["lat"]["set"][k])
    for k in ("candidates", "searches", "found", "search_rounds", "search_visited", "search_edges", "dead", "succ_edges", "prop_rounds", "prop_edges", "toggled"):
        out[f"set B/F: {k}"] = g(lambda e, k=k: e["lat"].get("set_bf", {}).get(k, 0))
    for k in ("dirty_states", "dirty_reached", "succ_calls", "edges_deleted", "edges_inserted", "unsupported", "toggled", "pred_checks"):
        out[f"Y2: {k}"] = g(lambda e, k=k: e["y2"][k])
    return out


def memory(res):
    out = []
    for m in res["memory"]:
        ws = m["worktrees"]
        tot = lambda f: sum(f(w) for w in ws)
        out.append({
            "at": m["at"], "worktrees": len(ws), "load_average": m["load_average"],
            "fwd_bytes": tot(lambda w: w["fwd"]["bytes"]), "fwd_bytes_tight": tot(lambda w: w["fwd"]["bytes_tight"]),
            "rev_bytes": tot(lambda w: w["rev"]["bytes"]), "rev_bytes_tight": tot(lambda w: w["rev"]["bytes_tight"]),
            "fwd_replaced": tot(lambda w: w["fwd"]["replaced"]),
            "fwd_replaced_flow_annotated_dispatch": [tot(lambda w, i=i: w["fwd"]["replaced_flow_annotated_dispatch"][i]) for i in range(3)],
            "rev_replaced": tot(lambda w: w["rev"]["replaced"]),
            "lvl_private": tot(lambda w: w["lvl_private"]), "set_private": tot(lambda w: w["set_private"]), "set_bf_private": tot(lambda w: w.get("set_bf_private", 0)), "seed_private": tot(lambda w: w["seed_private"]),
            "lvl_differ_sparse_bytes (8 B per differing id)": tot(lambda w: w["lvl_differ"]) * 8,
            "lvl_differ": tot(lambda w: w["lvl_differ"]), "lvl_pages_differ": tot(lambda w: w["lvl_pages_differ"]),
            "ext_ids": tot(lambda w: w["ext_ids"]), "ext_bytes": tot(lambda w: w["ext_bytes"]),
            "y2_dc_delta_bytes": tot(lambda w: w["y2_dc_bytes"]),
            "per_worktree_fwd_replaced": summ([w["fwd"]["replaced"] for w in ws]),
            "per_worktree_total_bytes": summ([w["fwd"]["bytes"] + w["rev"]["bytes"] + w["lvl_private"] + w["set_private"] + w["seed_private"] + w["ext_bytes"] for w in ws]),
        })
    return out


def by_divergence(res, at):
    m = next(x for x in res["memory"] if x["at"] == at)
    bins = [(0, 5), (6, 20), (21, 50), (51, 200), (201, 10**9)]
    out = []
    for lo, hi in bins:
        ws = [w for w in m["worktrees"] if lo <= w["divergence"] <= hi]
        if not ws:
            continue
        med = lambda f: pct([f(w) for w in ws], 0.5)
        out.append({"divergence": f"{lo}-{hi}" if hi < 10**9 else f">{lo - 1}", "worktrees": len(ws),
                    "fwd_replaced": med(lambda w: w["fwd"]["replaced"]), "fwd_dispatch": med(lambda w: w["fwd"]["replaced_flow_annotated_dispatch"][2]),
                    "fwd_kb": med(lambda w: w["fwd"]["bytes"]) / 1e3, "rev_kb": med(lambda w: w["rev"]["bytes"]) / 1e3,
                    "values_kb": med(lambda w: w["lvl_private"] + w["set_private"] + w["seed_private"]) / 1e3,
                    "lvl_differ": med(lambda w: w["lvl_differ"]), "ext_ids": med(lambda w: w["ext_ids"]), "y2_dc_kb": med(lambda w: w["y2_dc_bytes"]) / 1e3})
    return out


def main():
    d = Path(sys.argv[1])
    res = json.loads((d / "results.json").read_text())
    evs = json.loads((d / "events.json").read_text())
    s = {"exactness": res["exactness"], "base": res["base"], "latency": latency(evs), "counts": counts(evs), "memory": memory(res),
         "by_divergence_loaded": by_divergence(res, "loaded"), "by_divergence_end": by_divergence(res, "end"),
         "loads": {"create_ms": summ([l["create_ms"] for l in res["loads"]]), "rows_ms": summ([l["step"]["rows_ms"] for l in res["loads"]]), "lvl_ms": summ([l["step"]["lvl_ms"] for l in res["loads"]])},
         "seconds": res["seconds"], "copied_bytes": res.get("copied_bytes")}
    if "--json" in sys.argv:
        Path(sys.argv[sys.argv.index("--json") + 1]).write_text(json.dumps(s, indent=1))
    print("exactness:", res["exactness"]["checks"], "checks,", res["exactness"]["unequal"], "unequal")
    for pop, x in s["latency"].items():
        print(f"\n{pop} (n {x['y2_dc_ms']['n']}), ms p50 / p95 / p99 / max")
        for k, v in x.items():
            print(f"  {k:40s} {fmt(v)}")
    print("\ncounts per single-file edit, p50 / p95 / p99 / max")
    for k, v in s["counts"].items():
        print(f"  {k:40s} {fmt(v, 0) if isinstance(v['p50'], int) else fmt(v)}")
    print("\nmemory")
    for m in s["memory"]:
        print(" ", json.dumps({k: v for k, v in m.items() if not k.startswith("per_")}))
        print("   per worktree fwd replaced", fmt(m["per_worktree_fwd_replaced"], 0), "| total bytes", fmt(m["per_worktree_total_bytes"], 0))
    for at in ("loaded", "end"):
        print(f"\nby divergence ({at})")
        for r in s[f"by_divergence_{at}"]:
            print(" ", json.dumps(r))
    print("\nloads", json.dumps(s["loads"]))


if __name__ == "__main__":
    main()
