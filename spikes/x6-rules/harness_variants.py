#!/usr/bin/env python3
"""Policy variants for the per-file engine's stage harness (`facts-a repo` with `X6_STAGE_DUMP`;
exactness evidence only, original and rewrite treated alike).

Why: on `facts-a repo` a repository clause's inputs are spilled per file and loaded whole in
`eval::finish`. The rewrite's inputs are gate-free (`py_nodes`: every node of every file, ~16.5M
rows of 60+ columns with node texts), so its full-corpus repository run does not fit in memory
(measured: 1.39 GB RSS for the 219 files of `l2070_batching`, against 0.11 GB for the original).
Two checks replace it, both over every file of the 9,010:

- `perfile/`: each clause with `partitionBy: file`: every stage per content, on every file.
- `shard-<k>/`: the repository clause unchanged, its document's scope cut to a contiguous run of
  paths (≈ `--target` tape nodes each, all shards together = every file) plus, in every shard, the
  clause's `locations` files (quality-conditional-1's cross-file anchors). Every stage of each
  shard's repository run is compared.

state-flow-project-1 is taken in its live configuration (`linked/scan/`, see `scan_variant.py`).

usage: harness_variants.py PER_FILE_TSV [--target N]   (PER_FILE_TSV: a `facts-a repo` per-file.tsv
over the corpus with the quality document's scope, i.e. every .py file)"""
import argparse
import copy
import csv
import json
import re
from pathlib import Path

HERE = Path(__file__).parent
DOCS = {  # (side, policy file) -> source document
    ("original", "data-layer-serverplan-quality.json"): "plain/original/data-layer-serverplan-quality.json",
    ("rewrite", "data-layer-serverplan-quality.json"): "plain/rewrite/data-layer-serverplan-quality.json",
    ("original", "data-layer-serverplan-state-tripwires.json"): "scan/original/data-layer-serverplan-state-tripwires.json",
    ("rewrite", "data-layer-serverplan-state-tripwires.json"): "scan/rewrite/data-layer-serverplan-state-tripwires.json",
}


def glob_re(g):
    out, i = "", 0
    while i < len(g):
        if g.startswith("**/", i):
            out, i = out + "(?:.*/)?", i + 3
        elif g.startswith("**", i):
            out, i = out + ".*", i + 2
        elif g[i] == "*":
            out, i = out + "[^/]*", i + 1
        elif g[i] == "?":
            out, i = out + "[^/]", i + 1
        else:
            out, i = out + re.escape(g[i]), i + 1
    return re.compile(out + r"\Z")


def in_scope(scope, p):
    return any(glob_re(g).match(p) for g in scope["include"]) and not any(glob_re(g).match(p) for g in scope.get("exclude", []))


def rule_clause(doc):
    return next(c for c in doc["clauses"] if "relational" in c)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("per_file_tsv")
    ap.add_argument("--target", type=int, default=2_500_000)
    a = ap.parse_args()
    files = sorted((r["path"], int(r["nodes"])) for r in csv.DictReader(open(a.per_file_tsv), delimiter="\t"))
    shards, cur, n = [], [], 0
    for p, k in files:
        if cur and n + k > a.target:
            shards.append(cur)
            cur, n = [], 0
        cur.append(p)
        n += k
    shards.append(cur)
    root = HERE / "linked/harness"
    manifest = {"files": len(files), "target_nodes": a.target, "shards": []}
    for (side, name), src in DOCS.items():
        doc = json.loads((HERE / "linked" / src).read_text())
        d = copy.deepcopy(doc)
        rule_clause(d)["relational"]["partitionBy"] = "file"
        (root / "perfile" / side).mkdir(parents=True, exist_ok=True)
        (root / "perfile" / side / name).write_text(json.dumps(d, indent=1) + "\n")
        anchors = rule_clause(doc)["relational"].get("locations", [])
        for k, paths in enumerate(shards):
            d = copy.deepcopy(doc)
            keep = [p for p in dict.fromkeys(paths + anchors) if in_scope(doc["scope"], p)]
            d["scope"] = {"include": keep, **({"exclude": doc["scope"]["exclude"]} if "exclude" in doc["scope"] else {})}
            (root / f"shard-{k}" / side).mkdir(parents=True, exist_ok=True)
            (root / f"shard-{k}" / side / name).write_text(json.dumps(d, indent=1) + "\n")
            if side == "original":
                manifest["shards"].append({"shard": k, "policy": name, "files": len(keep), "first": paths[0], "last": paths[-1]})
    (root / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    print(len(shards), "shards")


if __name__ == "__main__":
    main()
