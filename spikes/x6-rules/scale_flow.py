#!/usr/bin/env python3
"""Dataflow cost vs the number of rules, on one worktree (the oracle tree, `greplay --parity`, no
events): for each N, N synthetic rules (`synth.py`) linked into one merged document with the
declared facts, exported by `facts-a xexport`, loaded into the dataflow with `FACTS_PROFILE=1`.
Records RSS at load, arranged records (total, and per stage class: linker `views/*`, shared facts
`proj_*`/`py_*`, rule stages `<alias>__*`) and the static plan size. N = 0 is the facts probe.

usage: scale_flow.py --facts F --mix-per-n "dc:{n}" --n 0,1,2,4,8 --out RUN_DIR [--probe RULE]"""
import argparse
import collections
import json
import os
import re
import subprocess
from pathlib import Path

HERE = Path(__file__).parent
WT = HERE.parent.parent
FA, FB = "/tmp/x6/facts-a", "/tmp/x6/facts-b"
JOBS = Path.home() / "rerail-bench/facts/phase-2/s3-crossfile/exports/oracle-b6-jobs.tsv"


def stage_class(stage):
    st = stage.split("/", 1)[1] if "/" in stage else ""
    if st.startswith("views/"):
        return "linker"
    if st.startswith(("proj_", "py_", "sel_")):
        return "facts"
    m = re.match(r"([a-z]+\d*)__", st)
    return f"rule:{m[1]}" if m else ("findings" if st == "" else "other")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--facts", required=True)
    ap.add_argument("--mix-per-n", required=True)
    ap.add_argument("--n", default="0,1,2,4,8")
    ap.add_argument("--probe", default=str(HERE / "pack/rules/facts-probe-symbols.json"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--keep-exports", action="store_true")
    ap.add_argument("--stages", default="reference", help="xexport --stages (engine: faster, no references)")
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    for n in [int(x) for x in a.n.split(",")]:
        d = out / f"n{n}"
        d.mkdir(exist_ok=True)
        rules = a.probe if n == 0 else subprocess.run(["python3", str(HERE / "synth.py"), "--mix", a.mix_per_n.format(n=n, h=n // 2), "--out-dir", str(d / "rules")],
                                                      capture_output=True, text=True, check=True).stdout.strip()
        doc = d / "pack.json"
        subprocess.run(["python3", str(HERE / "link.py"), "--facts", a.facts, "--rules", rules, "--mode", "merged", "--out", str(doc)], check=True)
        exp = d / "exports.jsonl"
        subprocess.run([FA, "xexport", "--rule", str(doc), "--clauses", "pack", "--jobs", str(JOBS), "--out", str(exp), "--stages", a.stages, "--threads", "8"],
                       check=True, capture_output=True)
        spec = d / "spec.json"
        spec.write_text(json.dumps([{"rule": str(doc), "clauses": ["pack"], "exports": str(exp)}]))
        env = {**os.environ, "FACTS_PROFILE": "1", "FACTS_B_BIN": FB, "FACTS_RUN_ROOT": str(d)}
        run = subprocess.run([str(WT / "spikes/facts-b/scripts/run.sh"), f"flow-n{n}", "--", "greplay", "--plans", "none", "--real", str(spec), "--parity", str(JOBS), "--no-oracle"],
                             env=env, cwd=WT, capture_output=True, text=True, check=True).stdout.strip().splitlines()[-1]
        res = json.loads((Path(run) / "results.json").read_text())
        prof = json.loads((Path(run) / "profile.json").read_text()) if (Path(run) / "profile.json").exists() else {"stages": []}
        by = collections.Counter()
        for s in prof["stages"]:
            by[stage_class(s["stage"])] += s["arranged"]
        rule_classes = {k: v for k, v in by.items() if k.startswith("rule:")}
        err = (Path(run) / "stderr.txt").read_text()
        row = {"n": n, "run": run, "plan_nodes": res["plans"], "rss_loaded_mb": next(x["rss_mb"] for x in res["rss"] if x["at"] == "loaded"),
               "max_rss_mb": int(re.search(r"(\d+)\s+maximum resident set size", err)[1]) >> 20,
               "arranged_total": res["arrangements"]["total_records"], "arranged_linker": by["linker"], "arranged_facts": by["facts"],
               "arranged_rules_total": sum(rule_classes.values()), "arranged_per_rule": sorted(rule_classes.values()),
               "arranged_other": by["other"] + by["findings"], "resident": res["resident"], "export_bytes": exp.stat().st_size,
               "refused_static": res.get("refused_static"), "runtime_refusals_live": res.get("runtime_refusals_live")}
        rows.append(row)
        print(json.dumps({k: v for k, v in row.items() if k != "arranged_per_rule"}), flush=True)
        if not a.keep_exports:
            exp.unlink()
        (out / "results.json").write_text(json.dumps(rows, indent=1) + "\n")


if __name__ == "__main__":
    main()
