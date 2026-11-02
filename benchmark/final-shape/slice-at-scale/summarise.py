#!/usr/bin/env python3
"""Tables of one run.sh run (docs/final-shape/notes/t18-slice-at-scale.md): per worktree, native's and the slice's
costs, their peak footprints, the slice's failures and the exactness verdicts of compare.txt.

    benchmark/final-shape/slice-at-scale/summarise.py <run dir>
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

MB = 1024 * 1024


def time_l(path: Path) -> dict:
    """`/usr/bin/time -l`: wall, user and sys seconds, max RSS and peak footprint bytes."""
    out = {}
    if not path.exists():
        return out
    text = path.read_text()
    m = re.search(r"([\d.]+) real\s+([\d.]+) user\s+([\d.]+) sys", text)
    if m:
        out["real"], out["user"], out["sys"] = map(float, m.groups())
    for key, label in (("maxrss", "maximum resident set size"), ("footprint", "peak memory footprint")):
        m = re.search(rf"(\d+)\s+{label}", text)
        if m:
            out[key] = int(m.group(1))
    return out


def first_workspace(path: Path) -> dict | None:
    if not path.exists():
        return None
    workspaces = json.loads(path.read_text()).get("workspaces", [])
    return workspaces[0] if workspaces else None


def compare_rows(run: Path) -> dict:
    """compare.txt's row per file: (worktree, scan | edits) -> its counts."""
    rows = {}
    path = run / "compare.txt"
    if not path.exists():
        return rows
    pattern = re.compile(
        r"^(\w+)\.(scan|edits)\.json: native (\d+) violations \((\d+) keys\), slice (\d+) keys, only native (\d+), "
        r"only slice (\d+); slice unknown (\d+) \((\d+) where native did not fail\)"
    )
    for line in path.read_text().splitlines():
        m = pattern.match(line)
        if m:
            name, kind, *numbers = m.groups()
            rows[(name, kind)] = dict(zip(("violations", "native", "slice", "only_native", "only_slice", "unknown", "unexplained"), map(int, numbers)))
    return rows


def s(ms):
    return "—" if ms is None else f"{ms / 1000:.1f}"


def ms(value):
    return "—" if value is None else f"{value:.0f}"


def main(run: Path) -> None:
    names = sorted({p.name for p in (run / "native").iterdir() if p.is_dir() and p.name != "findings"} |
                   {p.name for p in (run / "slice").iterdir() if p.is_dir() and p.name != "findings"})
    compared = compare_rows(run)
    print(f"Run `{run}`\n")
    for line in (run / "provenance.txt").read_text().splitlines():
        print(f"    {line}")
    print()
    print("| worktree | Python files | MB | native scan wall / CPU s | slice cold wall / CPU s | native edit p50 / p95 ms | slice edit p50 / p95 ms (CPU p50) | native / slice peak footprint MB | slice store MB | slice |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---|")
    buckets = []
    for name in names:
        native = first_workspace(run / "native" / name / "native.json")
        slice_ = first_workspace(run / "slice" / name / "slice.json")
        nt, st = time_l(run / "native" / name / "time.txt"), time_l(run / "slice" / name / "time.txt")
        files = slice_.get("pythonFiles") if slice_ else None
        mb = slice_["pythonBytes"] / MB if slice_ else None
        scan = (native or {}).get("fullScan", {})
        cold = (slice_ or {}).get("cold", {})
        nedit = (native or {}).get("edits", {})
        sedit = (slice_ or {}).get("edits", {})
        failed = (slice_ or {}).get("failed")
        status = "ok" if slice_ and not failed else (f"{failed['phase']}: {failed['message']}" if failed else "no output")
        print(
            f"| {name} | {files if files is not None else '—'} | {'—' if mb is None else f'{mb:.1f}'} | "
            f"{s(scan.get('wallMs'))} / {s(scan.get('cpuMs'))} | {s(cold.get('wallMs'))} / {s(cold.get('cpuMs'))} | "
            f"{ms(nedit.get('wallP50Ms'))} / {ms(nedit.get('wallP95Ms'))} | "
            f"{ms(sedit.get('wallP50Ms'))} / {ms(sedit.get('wallP95Ms'))} ({ms(sedit.get('cpuP50Ms'))}) | "
            f"{nt.get('footprint', 0) // MB or '—'} / {st.get('footprint', 0) // MB or '—'} | "
            f"{'—' if not slice_ or 'store' not in slice_ else slice_['store']['bytes'] // MB} | {status} |"
        )
        for sample in sedit.get("samples", []):
            buckets.append((name, sample))
    print()
    print("| worktree | findings | native violations (keys) | slice keys | only native | only slice | slice Unknown (native did not fail) | verdict |")
    print("|---|---|---:|---:|---:|---:|---:|---|")
    for name in names:
        for kind in ("scan", "edits"):
            row = compared.get((name, kind))
            if row is None:
                continue
            verdict = "equal" if row["only_native"] == 0 and row["only_slice"] == 0 else "DIFFER"
            print(
                f"| {name} | {kind} | {row['violations']} ({row['native']}) | {row['slice']} | {row['only_native']} | {row['only_slice']} | "
                f"{row['unknown']} ({row['unexplained']}) | {verdict} |"
            )
    if buckets:
        print()
        print("| worktree | edit bucket | file bytes | slice wall ms | slice CPU ms | evaluated | re-run |")
        print("|---|---|---:|---:|---:|---:|---:|")
        for name, sample in buckets:
            print(f"| {name} | {sample['bucket']} | {sample['sourceBytes']} | {sample['wallMs']:.0f} | {sample['cpuMs']:.0f} | {sample['evaluated']} | {sample['rerun']} |")
    loads = (run / "load.txt").read_text().strip() if (run / "load.txt").exists() else ""
    if loads:
        print("\nLoad (1, 5, 15 min) at each process's start and end:\n")
        for line in loads.splitlines():
            print(f"    {line}")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
