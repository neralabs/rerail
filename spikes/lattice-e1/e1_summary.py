#!/usr/bin/env python3
"""E1 tables from a sweep directory (`e1.sh sweep-*`) and its graph's `meta.json`.

    e1_summary.py SWEEP_DIR GRAPH_DIR [--json OUT]

Per configuration (file name a|b-<lattice>-<mode>-<threads>-r<k>.json): medians over the runs of
every repetition; speedups against the sequential kernel's median; exactness and digests across
thread counts.
"""
import json
import re
import statistics
import sys
from collections import defaultdict
from pathlib import Path


def med(v):
    return statistics.median(v) if v else float("nan")


def main():
    sweep, graph = Path(sys.argv[1]), Path(sys.argv[2])
    meta = json.loads((graph / "meta.json").read_text())
    rows = defaultdict(list)
    for f in sorted(sweep.glob("*.json")):
        m = re.match(r"([ab])-(\w+)-(\w+(?:-\w+)?)-(\d+)-r(\d+)\.json$", f.name)
        if not m:
            continue
        w, lat, mode, t, _ = m.groups()
        rows[(w, lat, mode, int(t))].append(json.loads(f.read_text()))
    out = {"sweep": str(sweep), "graph": str(graph), "configs": []}
    for (w, lat, mode, t), rs in sorted(rows.items()):
        wall = [x for r in rs for x in r["wall_ms"]]
        cpu = [x for r in rs for x in r["cpu_ms"]]
        la = [x for r in rs for x in r["load_average_1m"]]
        c = {"workload": w, "lattice": lat, "mode": mode, "threads": t, "reps": len(rs), "runs": len(wall), "wall_ms": med(wall), "wall_ms_min": min(wall),
             "cpu_ms": med(cpu), "load_1m": med(la), "exact": all(r["exact"] for r in rs),
             "peak_footprint_mb": max(r["memory"]["peak_footprint"] for r in rs) / 2**20, "max_rss_mb": max(r["memory"]["max_rss"] for r in rs) / 2**20,
             "mapped_mb": rs[0]["memory"]["mapped_bytes"] / 2**20, "copied_bytes": max(r["memory"]["copied_bytes"] for r in rs),
             "fixpoint_private_mb": rs[0]["fixpoint_private_bytes"] / 2**20, "private_per_worker_mb": rs[0].get("private_bytes_per_worker", 0) / 2**20}
        if w == "a":
            c["digests"] = sorted({d for r in rs for d in r["digests"]})
            c["rounds"] = rs[0]["counts"]["rounds"]
            c["edges_scanned"] = rs[0]["counts"]["edges_scanned"]
            waits = [statistics.mean(x) for r in rs for x in r.get("barrier_wait_ms", []) if x]
            c["barrier_wait_ms_per_worker"] = med(waits) if waits else 0.0
        else:
            c["digests"] = [x["digest"] for x in rs[0]["worktrees"]]
            c["worktrees"] = len(rs[0]["worktrees"])
            c["differences"] = sum(x["differences"] for r in rs for x in r["worktrees"])
            c["sum_fixpoint_ms"] = sum(x["fixpoint_ms_median"] for x in rs[0]["worktrees"])
        out["configs"].append(c)
    # Speedups against the sequential kernel; digest agreement across thread counts.
    for w in "ab":
        for lat in ("level", "set"):
            seq = [c for c in out["configs"] if c["workload"] == w and c["lattice"] == lat and c["mode"] in ("seq", "within-seq") and c["threads"] == 1]
            if not seq:
                continue
            s = seq[0]
            for c in out["configs"]:
                if c["workload"] == w and c["lattice"] == lat:
                    c["speedup_vs_seq"] = s["wall_ms"] / c["wall_ms"]
                    c["digests_equal_seq"] = c["digests"] == s["digests"]
    print(f"# {sweep}\n")
    for w in "ab":
        cs = [c for c in out["configs"] if c["workload"] == w]
        if not cs:
            continue
        print(f"## workload {w.upper()}\n")
        print("| lattice | mode | threads | runs | wall ms (median) | min | CPU ms | speedup vs seq | load 1m | peak private MB | max RSS MB | mapped MB | copied B | exact | digests = seq |" + (" barrier wait ms/worker |" if w == "a" else " Σ per-worktree ms |"))
        print("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
        for c in cs:
            extra = f" {c['barrier_wait_ms_per_worker']:.2f} |" if w == "a" else f" {c['sum_fixpoint_ms']:.1f} |"
            print(f"| {c['lattice']} | {c['mode']} | {c['threads']} | {c['runs']} | {c['wall_ms']:.2f} | {c['wall_ms_min']:.2f} | {c['cpu_ms']:.1f} | {c.get('speedup_vs_seq', float('nan')):.2f} | {c['load_1m']:.1f} | "
                  f"{c['peak_footprint_mb']:.1f} | {c['max_rss_mb']:.1f} | {c['mapped_mb']:.1f} | {c['copied_bytes']} | {c['exact']} | {c.get('digests_equal_seq')} |" + extra)
        print()
    # Y2's baseline on the same inputs (graph build), per worktree.
    ws = meta["worktrees"]
    y2 = {"reach_ms": sum(w["y2"]["reach_ms_median"] for w in ws), "succ_at_reached_ms": sum(w["y2"]["succ_at_reached_ms_median"] for w in ws),
          "findings_ms": sum(w["y2"]["findings_ms_median"] for w in ws), "enumerate_ms": sum(w["graph"]["enumerate_ms"] for w in ws),
          "assemble_ms": sum(w["graph"]["assemble_ms"] for w in ws), "load_1m": [w["y2"]["load_average_1m"][0] for w in ws][:3]}
    y2["traversal_ms"] = y2["reach_ms"] - y2["succ_at_reached_ms"]
    out["y2"] = y2
    print(f"## Y2 on the same inputs ({len(ws)} worktree(s), sums of per-worktree medians)\n")
    print("| reach ms | of which succ at reached states | traversal (reach − succ) | findings ms | CSR: enumerate ms | CSR: assemble ms |")
    print("|---|---|---|---|---|---|")
    print(f"| {y2['reach_ms']:.1f} | {y2['succ_at_reached_ms']:.1f} | {y2['traversal_ms']:.1f} | {y2['findings_ms']:.1f} | {y2['enumerate_ms']:.1f} | {y2['assemble_ms']:.1f} |\n")
    # Workload A per round: medians over every run of each kernel at 1 and 6 threads.
    per = {}
    for (w, lat, mode, t), rs in rows.items():
        if w == "a" and lat == "level":
            rm = [x for r in rs for x in r["round_ms"]]
            per[(mode, t)] = [med([x[k] for x in rm]) for k in range(len(rm[0]))]
            counts = rs[0]["counts"]
            per[("counts", mode)] = counts
    if ("seq", 1) in per:
        c = per[("counts", "seq")]
        cols = [k for k in [("seq", 1), ("par", 1), ("par", 6), ("edge", 6), ("par", 12)] if k in per]
        print("## Workload A, min_level, per round (ms: medians over runs)\n")
        print("| round | width | edges | changed | " + " | ".join(f"{m}@{t} ms" for m, t in cols) + " | par@6 speedup | edge@6 speedup |")
        print("|" + "---|" * (4 + len(cols) + 2))
        for k in range(len(c["width"])):
            v = [per[x][k] for x in cols]
            sp = lambda m: per[("seq", 1)][k] / per[(m, 6)][k] if (m, 6) in per and per[(m, 6)][k] > 0 else float("nan")
            print(f"| {k} | {c['width'][k]} | {c['edges'][k]} | {c['changed'][k]} | " + " | ".join(f"{x:.3f}" for x in v) + f" | {sp('par'):.2f} | {sp('edge'):.2f} |")
        out["per_round"] = {f"{m}@{t}": per[(m, t)] for m, t in cols}
        print()
    # Workload B per worktree.
    seqb = rows.get(("b", "level", "within-seq", 1))
    if seqb:
        wt = seqb[0]["worktrees"]
        rr = sorted(x["counts"]["rounds"] for x in wt)
        mw = sorted(x["counts"]["max_width"] for x in wt)
        es = sorted(x["counts"]["edges_scanned"] for x in wt)
        fx = sorted(x["fixpoint_ms_median"] for x in wt)
        rep = sorted(w["graph"]["replaced_rows"] for w in ws)
        q = lambda v: f"{v[0]} / {v[len(v) // 2]} / {v[-1]}"
        print("## Workload B per worktree (min / median / max)\n")
        print(f"- rounds {q(rr)}; max frontier width {q(mw)}; edges scanned {q(es)}; replaced rows {q(rep)}")
        print(f"- sequential fixpoint ms {fx[0]:.2f} / {fx[len(fx) // 2]:.2f} / {fx[-1]:.2f}; Y2 reach ms (median per worktree) "
              f"{min(w['y2']['reach_ms_median'] for w in ws):.1f} / {sorted(w['y2']['reach_ms_median'] for w in ws)[len(ws) // 2]:.1f} / {max(w['y2']['reach_ms_median'] for w in ws):.1f}\n")
    if len(sys.argv) > 4 and sys.argv[3] == "--json":
        Path(sys.argv[4]).write_text(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
