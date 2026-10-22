#!/usr/bin/env python3
"""Summarize Spike B run directories: median and spread per metric across repeated runs.

usage: summarize.py RUN_DIR [RUN_DIR ...]
Replay runs: single-file edit latency (p50/p95/p99), bulk, new worktree, commits, RSS at
'edit 1000' (steady state, 40 worktrees + 5 clones), peak RSS. Extract runs: per-file
parse+extract ms and wall/CPU.
"""
import json
import re
import statistics
import sys


def load(d):
    r = json.load(open(f"{d}/results.json"))
    st = open(f"{d}/stderr.txt").read()
    m = re.search(r"(\d+)\s+maximum resident set size", st)
    r["_peak_mb"] = int(m.group(1)) / 2**20 if m else None
    up = open(f"{d}/uptime.before").read().split("averages:")[-1].split()
    r["_load1"] = float(up[0].strip(","))
    return r


def med(vals):
    vals = [v for v in vals if v is not None]
    return f"{statistics.median(vals):.3f} [{min(vals):.3f}-{max(vals):.3f}]"


def replay(rs):
    out = {}
    lat = lambda g, k: [r["latency"][g][k] for r in rs if g in r["latency"]]
    for g in ["single_file_all", "edit", "new_worktree_edit", "bulk", "new_worktree", "commit", "commit_init"]:
        for k in ["p50_ms", "p95_ms", "p99_ms", "max_ms"]:
            v = lat(g, k)
            if v:
                out[f"{g}.{k}"] = med(v)
    rss = lambda at: [next((x["rss_mb"] for x in r["rss"] if x["at"] == at), None) for r in rs]
    for at in ["loaded", "edit 1000", "end"]:
        out[f"rss_mb@{at}"] = med(rss(at))
    out["peak_rss_mb"] = med([r["_peak_mb"] for r in rs])
    out["arranged_records"] = med([r["arrangements"]["total_records"] for r in rs])
    out["load.deltas_epoch_seconds"] = med([r["load"]["deltas_epoch_seconds"] for r in rs])
    out["load1_before"] = [r["_load1"] for r in rs]
    return out


def extract(rs):
    out = {}
    for k in ["p50", "p95", "p99", "max", "mean"]:
        out[f"per_file_ms.{k}"] = med([r["per_file_ms"][k] for r in rs])
    out["extract_wall_seconds"] = med([r["extract_wall_seconds"] for r in rs])
    out["parse_extract_cpu_seconds"] = med([r["parse_extract_cpu_seconds"] for r in rs])
    out["fact_bytes_per_content.mean"] = med([r["fact_bytes_per_content"]["mean"] for r in rs])
    out["load1_before"] = [r["_load1"] for r in rs]
    return out


if __name__ == "__main__":
    rs = [load(d) for d in sys.argv[1:]]
    s = extract(rs) if "per_file_ms" in rs[0] else replay(rs)
    for k, v in s.items():
        print(f"{k}: {v}")
