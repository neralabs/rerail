#!/usr/bin/env python3
"""Check the generated pack's rules against Ruff's own findings on Ruff's fixtures (main's
packs/python/upstream/ruff: fixtures/ and fixtures.expected.json, Ruff 0.16.9).

    ENGINE=<rerail-engine> python3 docs/rule-syntax/site/upstream_check.py <upstream dir> [CODE ...]

<upstream dir> holds fixtures/ and fixtures.expected.json (`git archive main packs/python/upstream/ruff`). Each
rule (every one the expectations name, or the CODEs given) runs alone over the fixtures, as run_main_pack.sh runs
it: the engine's `logic` command, under the caps (capped.sh: 2 minutes, 2 GB), RERAIL_SQL_STRICT=1, the repo's .rr
library. A finding is (path, line, column), the column from 1 as Ruff counts it. One line per rule:

    <code> <ok|differs|failed> expected=<n> got=<n> missing=<n> extra=<n>  <first few differences>

A dev tool: it is not part of the product.
"""
import json
import os
import subprocess
import sys
import tempfile

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.abspath(os.path.join(here, "..", "..", ".."))
pack = os.path.join(here, "..", "packs", "python-main")


def rule_file(code):
    prefix = f"ruff-{code.lower()}-"
    names = [n for n in os.listdir(pack) if n.startswith(prefix) and n.endswith(".rr")]
    return os.path.join(pack, names[0]) if len(names) == 1 else None


def run(engine, rr, fixtures):
    os.makedirs(os.path.join(root, "tmp"), exist_ok=True)
    d = tempfile.mkdtemp(prefix="upstream-check.", dir=os.path.join(root, "tmp"))
    os.mkdir(os.path.join(d, "r"))
    with open(rr) as src, open(os.path.join(d, "r", os.path.basename(rr)), "w") as dst:
        dst.write(src.read())
    env = dict(os.environ, RERAIL_TAPE_CACHE=os.path.join(d, "store.sqlite"),
               RERAIL_RR_LIB=os.environ.get("RERAIL_RR_LIB", os.path.join(root, "docs", "rule-syntax", "lib")))
    summary = subprocess.run([os.path.join(here, "capped.sh"), "120", "2048", os.path.join(d, "log"), engine, "logic",
                              os.path.join(d, "r"), fixtures, os.path.join(d, "o")],
                             env=env, capture_output=True, text=True).stdout.strip()
    found, error = None, ""
    csv = os.path.join(d, "o", "violation.csv")
    if summary.startswith("rc=0") and os.path.exists(csv):
        found = set()
        for line in open(csv):
            cols = line.rstrip("\n").split("\t")
            found.add((cols[3], int(cols[4]), int(cols[5]) + 1))
    else:
        for line in open(os.path.join(d, "log"), errors="replace"):
            if "rerail-engine:" in line or "panicked" in line:
                error = line.strip()[:200]
                break
        error = error or summary
    # the run's dir moves to the trash, as run_main_pack.sh's do
    os.makedirs("/tmp/ai-trash", exist_ok=True)
    os.rename(d, os.path.join("/tmp/ai-trash", os.path.basename(d)))
    return found, error


def main():
    engine = os.environ.get("ENGINE") or sys.exit("set ENGINE to the rerail-engine binary")
    upstream = sys.argv[1]
    expected = json.load(open(os.path.join(upstream, "fixtures.expected.json")))
    codes = sys.argv[2:] or expected["source"]["rules"]
    for code in codes:
        want = {(f["path"], f["line"], f["column"]) for f in expected["findings"] if f["rule"] == code}
        rr = rule_file(code)
        if rr is None:
            print(f"{code} failed  no single generated rule ruff-{code.lower()}-*.rr", flush=True)
            continue
        got, error = run(engine, rr, os.path.join(upstream, "fixtures"))
        if got is None:
            print(f"{code} failed expected={len(want)}  {error}", flush=True)
            continue
        missing, extra = sorted(want - got), sorted(got - want)
        status = "ok" if not missing and not extra else "differs"
        sample = " ".join(f"-{p}:{l}:{c}" for p, l, c in missing[:3]) + " " + " ".join(f"+{p}:{l}:{c}" for p, l, c in extra[:3])
        print(f"{code} {status} expected={len(want)} got={len(got)} missing={len(missing)} extra={len(extra)}  {sample.strip()}",
              flush=True)


if __name__ == "__main__":
    main()
