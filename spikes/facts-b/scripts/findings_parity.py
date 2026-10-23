#!/usr/bin/env python3
"""Clause parity: a greplay parity run's findings rows for one clause (path, start_byte, end_byte, …,
with the code column named by --code-col) against the oracle suite's findings for its check, as
multisets of (path, line, code); a finding's line is its start byte's 1-based line in the job's source.

usage: findings_parity.py RUN_DIR --clause PLAN:CLAUSE --check CHECK --jobs JOBS_TSV [--suite SUITE_JSON]
       [--code-col N]   (N: index of the code column in the row; default: no code column, code = the check)
Writes RUN_DIR/findings-parity-<clause>.json; exit 1 on any difference or live runtime refusal."""
import argparse
import bisect
import collections
import json
from pathlib import Path

from edges_parity import parse


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("--clause", required=True)
    ap.add_argument("--check", required=True)
    ap.add_argument("--jobs", required=True)
    ap.add_argument("--suite", default="/tmp/dl60/suite-b5.json")
    ap.add_argument("--code-col", type=int)
    a = ap.parse_args()
    run = Path(a.run)
    res = json.loads((run / "results.json").read_text())
    rows = res["parity_rows"].get(a.clause)
    if rows is None:
        raise SystemExit(f"no parity rows for {a.clause}: {sorted(res['parity_rows'])}")
    sources = {}
    for line in Path(a.jobs).read_text().splitlines():
        if line.strip():
            path, _hash, src = line.split("\t")
            sources[path] = src
    starts = {}

    def line_of(path, byte):
        if path not in starts:
            data = Path(sources[path]).read_bytes()
            starts[path] = [0] + [i + 1 for i, b in enumerate(data) if b == 10]
        return bisect.bisect_right(starts[path], byte)

    got = collections.Counter()
    for r in rows:
        v = parse(r)
        code = v[a.code_col] if a.code_col is not None else a.check
        got[(v[0], line_of(v[0], v[1]), code)] += 1
    suite = json.loads(Path(a.suite).read_text())
    events = [e for e in suite["events"] if e["check"] == a.check]
    want = collections.Counter()
    for e in events:
        for f in e["findings"]:
            want[(f["path"], f["line"], f["code"] if a.code_col is not None else a.check)] += 1
    missing, extra = want - got, got - want
    live = res.get("runtime_refusals_live") or []
    out = {"run": str(run), "clause": a.clause, "check": a.check, "suite": a.suite, "generator": sum(got.values()), "oracle": sum(want.values()),
           "missing": sum(missing.values()), "extra": sum(extra.values()), "first_missing": [list(k) for k in list(missing)[:10]],
           "first_extra": [list(k) for k in list(extra)[:10]], "runtime_refusals_live": live, "refused_static": res.get("refused_static")}
    name = a.clause.split(":")[-1]
    (run / f"findings-parity-{name}.json").write_text(json.dumps(out, indent=1))
    print(json.dumps({k: v for k, v in out.items() if k != "refused_static"}, indent=1))
    raise SystemExit(0 if not missing and not extra and not live else 1)


if __name__ == "__main__":
    main()
