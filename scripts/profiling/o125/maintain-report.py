#!/usr/bin/env python3
"""maintain-report.py DIR: the O125 replay's measurement (maintain-measure.sh) as a table (t43 step 1, dev tool).

DIR holds `kept/`, `session/` and `cold/`. Per step: each mode's instructions retired and peak footprint, the
frames it queued and the kept part's bytes (RERAIL_KEPT_BYTES), and whether its findings are the cold run's (R10).
"""
import json, re, sys
from pathlib import Path

d = Path(sys.argv[1])


def findings(text):
    """Every JSON document in `text` (a session's commands print one each), as its findings' sets."""
    out, i, dec = [], 0, json.JSONDecoder()
    while True:
        while i < len(text) and text[i].isspace():
            i += 1
        if i >= len(text):
            return out
        doc, i = dec.raw_decode(text, i)
        out.append({(f["path"], f["line"], f["code"]) for f in doc.get("diagnostics", [])})


def err_figures(text):
    g = lambda pat: (m.group(1) if (m := re.search(pat, text)) else "")
    return {
        "instructions": g(r"(\d+)\s+instructions retired"),
        "footprint": g(r"(\d+)\s+peak memory footprint"),
        "frames": g(r"frames queued: ([\d.]+ MB in \d+ frames of \d+ relations)"),
            }


steps = sorted(p.stem for p in (d / "cold").glob("[0-9][0-9].json"))
cold = {s: findings((d / "cold" / f"{s}.json").read_text())[0] for s in steps}
session = findings((d / "session" / "all.json").read_text()) if (d / "session" / "all.json").exists() else []
session_lines = re.findall(r"repeat line \d+: ([\d.]+) s CPU[^;]*; footprint (\d+) MB, at most (\d+) MB; (\w+)", (d / "session" / "all.err").read_text()) if (d / "session" / "all.err").exists() else []
session_runs = [l for l in session_lines]
print("step | cold instr | kept instr | kept = cold | kept frames | session CPU s | session = cold")
for k, s in enumerate(steps):
    c = err_figures((d / "cold" / f"{s}.err").read_text())
    kp = d / "kept" / f"{s}.err"
    ke = err_figures(kp.read_text()) if kp.exists() else {}
    kf = findings((d / "kept" / f"{s}.json").read_text())[0] if (d / "kept" / f"{s}.json").exists() else None
    sf = session[k] if k < len(session) else None
    sc = session_runs[k][0] if k < len(session_runs) else ""
    same = lambda f: "-" if f is None else ("yes" if f == cold[s] else f"NO ({len(f - cold[s])} extra, {len(cold[s] - f)} missing)")
    print(f"{s} | {c['instructions']} | {ke.get('instructions', '')} | {same(kf)} | {ke.get('frames', '')} | {sc} | {same(sf)}")
# The kept part's size, the shape counters (cumulative in a session), and the strata by how they ran.
for mode in ("kept", "session"):
    for p in sorted((d / mode).glob("*.err")):
        text = p.read_text()
        for line in text.splitlines():
            if line.startswith("KEPT ") or line.startswith("SHAPE ") or line.startswith("repeat line"):
                print(f"{mode}/{p.name}: {line[:240]}")
        # RERAIL_MODE_TRACE: per run, each reached stratum's handling; a run's first line is its first stratum.
        why = {}
        for m in re.finditer(r"RERAIL_MODE_TRACE (\S+): ([^\n]+)", text):
            # Its reason, without the sizes the trace ends with (`(N delta rows, M kept)`).
            w = re.sub(r" \(\d+ delta rows, \d+ kept\)$", "", m.group(2))
            why[w] = why.get(w, 0) + 1
        if why:
            print(f"{mode}/{p.name} strata by handling: " + "; ".join(f"{w}: {n}" for w, n in sorted(why.items(), key=lambda x: -x[1])))
