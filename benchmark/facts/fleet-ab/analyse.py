#!/usr/bin/env python3
"""Summarise a run-fleet-ab.sh run directory: native vs facts on 40 worktrees (markdown to stdout)."""
import json, re, sys, statistics
from pathlib import Path

run = Path(sys.argv[1])


def time_l(p: Path) -> dict:
    t = p.read_text()
    real, user, sys_ = map(float, re.search(r"([\d.]+) real\s+([\d.]+) user\s+([\d.]+) sys", t).groups())
    maxrss = int(re.search(r"(\d+)\s+maximum resident set size", t).group(1))
    return {"real": real, "cpu": user + sys_, "maxrss": maxrss}


def pct(xs, q):
    xs = sorted(xs)
    if not xs:
        return float("nan")
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def dist(xs):
    return f"{pct(xs, .5):,.1f} / {pct(xs, .95):,.1f} / {max(xs):,.1f}"


GiB = 1024 ** 3
out = []

# native
nat = sorted((run / "native").glob("[0-9][0-9]/native.json"))
nscan_wall, nscan_cpu, nedit, npeak, nviol, ncpu_proc = [], [], [], [], [], []
nedit_bucket = {}
for f in nat:
    w = json.loads(f.read_text())["workspaces"][0]
    nscan_wall.append(w["fullScan"]["wallMs"] / 1000)
    nscan_cpu.append(w["fullScan"]["cpuMs"] / 1000)
    nviol.append(w["fullScan"]["violations"])
    npeak.append(w["rss"]["peak"])
    for s in w["edits"]["samples"]:
        nedit.append(s["wallMs"])
        nedit_bucket.setdefault(s["bucket"], []).append(s["wallMs"])
    ncpu_proc.append(time_l(f.parent / "time.txt")["cpu"])
fleet_wall = int(re.search(r"(\d+)", (run / "native/fleet-wall.txt").read_text()).group(1))

# facts
def facts(d):
    j = json.loads((run / d / "facts.json").read_text())
    ws = j["workspaces"]
    sync = [w["sync"]["wallMs"] / 1000 for w in ws]
    edits = [s["wallMs"] for w in ws for s in w["edits"]["samples"]]
    buckets = {}
    for w in ws:
        for s in w["edits"]["samples"]:
            buckets.setdefault(s["bucket"], []).append(s["wallMs"])
    rss = [w["rss"]["afterSync"] for w in ws] + [w["rss"]["afterEdits"] for w in ws] + [j["rss"]["steady"]]
    pushes = [p["ms"] / 1000 for p in j.get("pushes", []) if p.get("op") == "findings" and "ms" in p]
    findings = [w["sync"]["findings"] for w in ws]
    return {"j": j, "n": len(ws), "sync": sync, "edits": edits, "buckets": buckets, "rss": rss, "pushes": pushes,
            "findings": findings, "time": time_l(run / d / "time.txt")}

ff, fe = facts("facts-full"), facts("facts-edits")

out.append(f"Run `{run}`; worktrees native {len(nat)}, facts {ff['n']} / {fe['n']}.\n")
out.append("| | Native (4 processes at a time, store off) | Facts, full coverage (`--cross scratch`, 4 threads) | Facts, per-file only (`--cross none`, 4 threads) |")
out.append("|---|---|---|---|")
out.append(f"| Rules evaluated | 60 | 58 (2 refused by name) | 58, per-file clauses only (529 of 558) |")
out.append(f"| Fleet cold scan, wall | {fleet_wall:,} s | {ff['time']['real']:,.0f} s | {fe['time']['real']:,.0f} s (incl. 800 edits) |")
out.append(f"| Fleet CPU (user+sys, whole process tree) | {sum(ncpu_proc):,.0f} s | {ff['time']['cpu']:,.0f} s | {fe['time']['cpu']:,.0f} s (incl. 800 edits) |")
out.append(f"| Scan per worktree, wall p50 / p95 / max | {dist(nscan_wall)} s | {dist(ff['sync'])} s (per-file sync) | {dist(fe['sync'])} s |")
out.append(f"| Native scan CPU per worktree p50 / p95 / max | {dist(nscan_cpu)} s | | |")
if ff["pushes"]:
    out.append(f"| Cross-file pass per change (from scratch), p50 / p95 / max | in the scan and every edit | {dist(ff['pushes'])} s ({len(ff['pushes'])} pushes) | not run |")
out.append(f"| Edit latency, wall p50 / p95 / max | {dist(nedit)} ms ({len(nedit)} edits, all 60 rules) | (≈ one cross-file pass) | {dist(fe['edits'])} ms ({len(fe['edits'])} edits, per-file findings) |")
for b in ("minimum", "p50", "p95", "maximum"):
    if b in nedit_bucket or b in fe["buckets"]:
        out.append(f"| — edits to the {b}-size file | {dist(nedit_bucket.get(b, []))} ms | | {dist(fe['buckets'].get(b, []))} ms |")
out.append(f"| Memory peak | {max(npeak)/GiB:.1f} GiB per worktree process (p50 {statistics.median(npeak)/GiB:.1f}); 40 resident ≈ {sum(npeak)/GiB:,.0f} GiB [sum of per-process peaks] | {max(ff['rss'])/GiB:.2f} GiB daemon RSS (max sampled) | {max(fe['rss'])/GiB:.2f} GiB daemon RSS (max sampled) |")
out.append(f"| Findings per worktree (scan) | {min(nviol):,}–{max(nviol):,} (all rules) | {min(ff['findings']):,}–{max(ff['findings']):,} per-file at sync | {min(fe['findings']):,}–{max(fe['findings']):,} per-file |")
out.append("")
for name in ("compare-full.txt", "compare-edits.txt"):
    p = run / name
    if p.exists():
        lines = p.read_text().strip().splitlines()
        out.append(f"`{name}`: {lines[-1] if lines else '(empty)'}")
print("\n".join(out))
print("\nLoad:\n" + (run / "load.txt").read_text())
