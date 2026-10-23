#!/usr/bin/env python3
"""Exactness of a rewrite against its original on one snapshot: every output of the rewrite's parity
run (`greplay --parity` with `FACTS_STAGE_OUTPUTS=1`) is compared, as a multiset of rows, with the
original run's output it stands for. Names map by the pack's renames: `proj_X`/`py_X` stand for the
original stage `X` (and `<alias>__X` in a merged pack for the rule's stage `X`); a fact read by two
rules is compared with each original. Stages of the original with no counterpart (identity copies
of an input, dropped by the rewrite) are listed, not compared.

usage: stages_equal.py ORIGINAL_RUN REWRITE_RUN --map rewrite_plan=original_plan[:alias] ... [--out F]"""
import argparse
import collections
import json
from pathlib import Path


def outputs(run):
    return json.loads((Path(run) / "results.json").read_text())["parity_rows"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("original")
    ap.add_argument("rewrite")
    ap.add_argument("--map", nargs="+", required=True, help="rewrite_plan=original_plan[:alias]")
    ap.add_argument("--out")
    a = ap.parse_args()
    orig, new = outputs(a.original), outputs(a.rewrite)
    report = {"original": a.original, "rewrite": a.rewrite, "compared": [], "differ": [], "unmatched_original": [], "unmatched_rewrite": []}
    matched_new = set()
    for m in a.map:
        rplan, rest = m.split("=")
        oplan, _, alias = rest.partition(":")
        prefix = f"{rplan}:{rplan}"
        for okey, orows in orig.items():
            if not okey.startswith(f"{oplan}:{oplan}"):
                continue
            tail = okey[len(f"{oplan}:{oplan}"):]  # '' (findings) or '/stage/<name>'
            if tail == "":
                cands = [f"{prefix}/stage/{alias}__findings"] if alias else [prefix]
            else:
                st = tail.split("/stage/")[1]
                cands = [f"{prefix}/stage/{n}" for n in ([f"{alias}__{st}"] if alias else []) + [st, f"proj_{st}", f"py_{st}"]]
            hit = next((c for c in cands if c in new), None)
            if hit is None:
                report["unmatched_original"].append(okey)
                continue
            matched_new.add(hit)
            a_, b_ = collections.Counter(orows), collections.Counter(new[hit])
            row = {"original": okey, "rewrite": hit, "rows": sum(a_.values()), "missing": sum((a_ - b_).values()), "extra": sum((b_ - a_).values())}
            report["compared"].append(row)
            if row["missing"] or row["extra"]:
                row["first_missing"] = list((a_ - b_))[:3]
                row["first_extra"] = list((b_ - a_))[:3]
                report["differ"].append(row)
    report["unmatched_rewrite"] = sorted(set(new) - matched_new)
    text = json.dumps(report, indent=1)
    if a.out:
        Path(a.out).write_text(text + "\n")
    print(json.dumps({"compared": len(report["compared"]), "rows": sum(r["rows"] for r in report["compared"]), "differ": report["differ"],
                      "unmatched_original": report["unmatched_original"], "unmatched_rewrite": report["unmatched_rewrite"]}, indent=1))
    raise SystemExit(1 if report["differ"] else 0)


if __name__ == "__main__":
    main()
