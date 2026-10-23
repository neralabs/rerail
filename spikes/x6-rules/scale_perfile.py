#!/usr/bin/env python3
"""Per-file engine cost vs the number of rules: `facts-a xexport` (the per-content producer of the
repository rules: matchers, per-content stages and exported relations, Spike A's leaf executor and
the reference evaluator) over the oracle tree's 9,010 contents, for the facts probe alone (N = 0:
every declared per-content fact computed once and its keys exported) and for N synthetic rules
(`synth.py`) linked into one document (facts computed once per content, each rule a reader).
`--unshared`: each rule its own copy of every fact (`unshare.py`), today's shape.
Reports user/sys/wall seconds, max RSS, the exported bytes and the compile time of the document.

usage: scale_perfile.py --facts F[,F2] --mix-per-n "dc:{h},ef:{h}" --n 0,20,50,100,200 --out RUN_DIR
       [--threads 8] [--jobs JOBS] [--unshared]"""
import argparse
import json
import re
import subprocess
import time
from pathlib import Path

HERE = Path(__file__).parent
FA, FB = "/tmp/x6/facts-a", "/tmp/x6/facts-b"


def timed(cmd):
    t = time.time()
    p = subprocess.run(["/usr/bin/time", "-l", *cmd], capture_output=True, text=True)
    err = p.stderr
    if p.returncode != 0:
        raise SystemExit(f"{cmd[:3]} failed: {err[-2000:]}")
    m = re.search(r"([\d.]+) real\s+([\d.]+) user\s+([\d.]+) sys", err)
    rss = int(re.search(r"(\d+)\s+maximum resident set size", err)[1])
    return {"wall": float(m[1]), "user": float(m[2]), "sys": float(m[3]), "max_rss_mb": rss >> 20, "elapsed": time.time() - t}, p.stdout, err


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--facts", required=True)
    ap.add_argument("--mix-per-n", required=True, help='format with h = n/2, e.g. "dc:{h},ef:{h}"')
    ap.add_argument("--n", default="0,20,50,100,200")
    ap.add_argument("--probe", default=str(HERE / "pack/rules/facts-probe-symbols.json"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--threads", default="8")
    ap.add_argument("--jobs", default=str(Path.home() / "rerail-bench/facts/phase-2/s3-crossfile/exports/oracle-b6-jobs.tsv"))
    ap.add_argument("--unshared", action="store_true")
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    for n in [int(x) for x in a.n.split(",")]:
        d = out / f"n{n}"
        d.mkdir(exist_ok=True)
        facts = a.facts
        if n == 0:
            rules = a.probe
        else:
            rules = subprocess.run(["python3", str(HERE / "synth.py"), "--mix", a.mix_per_n.format(h=n // 2, n=n), "--out-dir", str(d / "rules")],
                                   capture_output=True, text=True, check=True).stdout.strip()
            if a.unshared:
                ff, rules = subprocess.run(["python3", str(HERE / "unshare.py"), "--facts", a.facts, "--rules", rules, "--out-dir", str(d / "unshared")],
                                           capture_output=True, text=True, check=True).stdout.split()
                facts = f"{a.facts},{ff}"
        doc = d / "pack.json"
        subprocess.run(["python3", str(HERE / "link.py"), "--facts", facts, "--rules", rules, "--mode", "merged", "--out", str(doc)], check=True)
        spec = d / "spec.json"
        spec.write_text(json.dumps([{"rule": str(doc), "clauses": ["pack"], "exports": "/nonexistent"}]))
        g, gout, _ = timed([FB, "gnorm", "--real", str(spec)])
        exp = d / "exports.jsonl"
        x, _, _ = timed([FA, "xexport", "--rule", str(doc), "--clauses", "pack", "--jobs", a.jobs, "--out", str(exp), "--stages", "reference", "--threads", a.threads])
        summ = json.loads((Path(str(exp) + ".summary.json")).read_text())
        row = {"n": n, "unshared": a.unshared, "rules": len(rules.split(",")) if n else 0, "doc_bytes": doc.stat().st_size, "gnorm": g,
               "gnorm_out": gout.strip()[-200:], "xexport": x, "export_bytes": exp.stat().st_size, "exports": len(summ["exports"]),
               "inputs": summ["inputs"], "failures": summ["failures"], "xexport_seconds": summ.get("seconds")}
        rows.append(row)
        print(json.dumps(row), flush=True)
        exp.unlink()  # keep the summary; the rows are regenerated on demand
        (out / "results.json").write_text(json.dumps(rows, indent=1) + "\n")


if __name__ == "__main__":
    main()
