#!/usr/bin/env python3
"""Exactness of a rewrite against its original through the per-file engine (`facts-a repo` with
`X6_STAGE_DUMP=DIR`, `spikes/facts-a/src/x6dump.rs`): every table the original's run stored or
read is compared, as a row multiset (row count and the 128-bit sum of per-row digests), with the
rewrite's table of the same name, or `proj_<name>`/`py_<name>`. The original's `syntax_rows_0..k`
(the dump union, split for stage-size limits) are compared, summed, with the rewrite's
`syntax_rows`. The original's dump inputs (`syntax_k`) and the rewrite's fact inputs have no
counterpart: listed, not compared.

- repository runs: `DIR/<label>.json`, one run;
- `--perfile`: `partitionBy: file` runs, `DIR/<label>.perfile.*.jsonl`, one line per content; each
  stage is compared per content (a content missing on one side counts as a difference) and summed.

usage: dump_equal.py ORIGINAL_DIR REWRITE_DIR LABEL [--perfile] [--out F]   (LABEL = <check>__<clause>)"""
import argparse
import collections
import json
import re
from pathlib import Path

M = 1 << 128


def load(d, label):
    return {t["name"]: (t["rows"], int(t["hash"], 16), t["cols"]) for t in json.loads((Path(d) / f"{label}.json").read_text())}


def load_perfile(d, label):
    out = {}
    for f in sorted(Path(d).glob(f"{label}.perfile.*.jsonl")):
        for line in f.open():
            r = json.loads(line)
            assert r["path"] not in out, r["path"]
            out[r["path"]] = {n: (rows, int(h, 16), None) for n, rows, h in r["tables"]}
    return out


def pairs(orig, new):
    """(original names, rewrite name or None) for every original table."""
    split = sorted((n for n in orig if re.fullmatch(r"syntax_rows_\d+", n)), key=lambda n: int(n.rsplit("_", 1)[1]))
    out = [(split, "syntax_rows")] if split and "syntax_rows" in new else []
    return out + [([n], next((c for c in (n, "proj_" + n, "py_" + n) if c in new), None)) for n in orig if n not in split]


def compare(orig, new):
    rows = []
    for names, cand in pairs(orig, new):
        if cand is None:
            rows.append({"original": "+".join(names), "rewrite": None})
            continue
        o = (sum(orig[n][0] for n in names), sum(orig[n][1] for n in names) % M)
        r = new[cand]
        cols = None if orig[names[0]][2] is None else all(orig[n][2] == r[2] for n in names)
        rows.append({"original": "+".join(names), "rewrite": cand, "rows": o[0], "rewrite_rows": r[0],
                     "equal": o == (r[0], r[1]) and cols is not False, **({"cols_equal": cols} if cols is not None else {})})
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("original")
    ap.add_argument("rewrite")
    ap.add_argument("label")
    ap.add_argument("--perfile", action="store_true")
    ap.add_argument("--out")
    a = ap.parse_args()
    rep = {"original": a.original, "rewrite": a.rewrite, "label": a.label}
    if not a.perfile:
        orig, new = load(a.original, a.label), load(a.rewrite, a.label)
        cmp = compare(orig, new)
        rep["compared"] = [c for c in cmp if c["rewrite"]]
        rep["differ"] = [c for c in rep["compared"] if not c["equal"]]
        rep["unmatched_original"] = [c["original"] for c in cmp if not c["rewrite"]]
        rep["unmatched_rewrite"] = sorted(set(new) - {c["rewrite"] for c in cmp})
    else:
        orig, new = load_perfile(a.original, a.label), load_perfile(a.rewrite, a.label)
        rep["contents"] = {"original": len(orig), "rewrite": len(new), "only_original": sorted(set(orig) - set(new))[:20], "only_rewrite": sorted(set(new) - set(orig))[:20]}
        per = collections.defaultdict(lambda: {"rows": 0, "rewrite_rows": 0, "contents_differ": 0, "first_paths": []})
        unmatched_o, unmatched_r = set(), set()
        for p in sorted(set(orig) | set(new)):
            o, n = orig.get(p, {}), new.get(p, {})
            for c in compare(o, n):
                if not c["rewrite"]:
                    unmatched_o.add(c["original"])
                    continue
                s = per[(c["original"], c["rewrite"])]
                s["rows"] += c["rows"]
                s["rewrite_rows"] += c["rewrite_rows"]
                if not c["equal"]:
                    s["contents_differ"] += 1
                    if len(s["first_paths"]) < 5:
                        s["first_paths"].append(p)
            unmatched_r |= set(n) - {c["rewrite"] for c in compare(o, n)}
            if not o or not n:  # a content one side did not run: every stage of the other differs
                for name in (o or n):
                    per[("<content run>", "<content run>")]["contents_differ"] += 1
        rep["compared"] = [{"original": k[0], "rewrite": k[1], **v, "equal": v["contents_differ"] == 0} for k, v in per.items()]
        rep["differ"] = [c for c in rep["compared"] if not c["equal"]]
        rep["unmatched_original"], rep["unmatched_rewrite"] = sorted(unmatched_o), sorted(unmatched_r)
    if a.out:
        Path(a.out).write_text(json.dumps(rep, indent=1) + "\n")
    print(json.dumps({k: v for k, v in rep.items() if k != "compared"} | {"compared": len(rep["compared"]), "rows": sum(c["rows"] for c in rep["compared"] if "rows" in c)}, indent=1))
    raise SystemExit(1 if rep["differ"] else 0)


if __name__ == "__main__":
    main()
