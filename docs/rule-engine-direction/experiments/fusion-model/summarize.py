"""Static layer classification and fusion-opportunity tables for REPORT.md.

Usage: python3 -I summarize.py RULES_DIR OUT_DIR   (needs out/queries.json, out/rules.json, out/run_A.json)
"""
import json
import os
import re
import sys
from collections import Counter, defaultdict

RULES, OUT = sys.argv[1], sys.argv[2]
Q = [q for q in json.load(open(os.path.join(OUT, "queries.json"))) if q["active"]]
RUN = json.load(open(os.path.join(OUT, "run_full.json" if os.path.exists(os.path.join(OUT, "run_full.json")) else "run_A.json")))

FACT = {"count", "compare", "layout", "binds", "calls", "arguments", "parse", "host", "resolves"}


def layer(q):
    sem = set(q["semantic"])
    scan = any(s[1] != "neighbor" for s in q["sites"])
    if q["role"] == "relational":
        base = "SQL-input"
    else:
        base = "rule"
    if sem & FACT:
        lay = "fact-family"
    elif q["recursive_utils"]:
        lay = "set-stage(recursive,CPU-only)"
    elif scan:
        lay = "set-stage"
    else:
        lay = "instruction-program"
    return base, lay


out = {}
lay = Counter(layer(q) for q in Q)
out["query_layers"] = {f"{a}/{b}": c for (a, b), c in sorted(lay.items())}
fact_keys = Counter(k for q in Q for k in q["semantic"] if k in FACT)
out["fact_keys_queries"] = {k: sum(1 for q in Q if k in q["semantic"]) for k in fact_keys}

# per rule: layers used
per_rule = defaultdict(Counter)
for q in Q:
    per_rule[q["rule"]][layer(q)[1]] += 1
rules = json.load(open(os.path.join(OUT, "rules.json")))
rule_layers = Counter()
for r in rules:
    ls = set(per_rule[r["rule"]])
    if r["file_regex_clauses"]:
        ls.add("text-regex")
    if r["clause_kinds"].get("relational"):
        ls.add("SQL-composition")
    top = ("SQL-composition" if "SQL-composition" in ls else "fact-family" if "fact-family" in ls else
           "set-stage(recursive,CPU-only)" if "set-stage(recursive,CPU-only)" in ls else "set-stage" if "set-stage" in ls else
           "instruction-program" if "instruction-program" in ls else "text-regex-only")
    rule_layers[top] += 1
out["rules_by_heaviest_layer"] = dict(rule_layers)

# relational clauses under decision 0002
rel = []
for fn in sorted(os.listdir(RULES)):
    d = json.load(open(os.path.join(RULES, fn)))
    for c in d["clauses"]:
        if "relational" not in c:
            continue
        r = c["relational"]
        txt = json.dumps(r.get("query")) + json.dumps(r.get("stages", []))
        T = txt.upper()
        cross = r.get("partitionBy") != "file"
        feats = {
            "cross_file": cross,
            "inputs": len(r["inputs"]),
            "stages": len(r.get("stages", [])),
            "syntax_relation": "syntax" in r,
            "references": "references" in r,
            "exists": "EXISTS" in T,
            "not_exists": "NOT EXISTS" in T,
            "join": " JOIN " in T or bool(re.search(r"FROM \w+ \w+\s*,", T)),
            "union": "UNION" in T,
            "group": "GROUP BY" in T,
            "udf_regex": "RX_" in T,
            "recursive_cte": "RECURSIVE" in T,
        }
        if feats["references"]:
            target = "fact family (reference graph) + SQL join"
        elif feats["syntax_relation"]:
            target = "engine relation `syntax` (node table) + SQL composition"
        elif cross:
            target = "cross-file SQL composition (require_each / select_when over t_export)"
        elif feats["not_exists"] or feats["exists"]:
            target = "per-file semi/anti join -> fuse into one instruction program (EXISTS/NOT_EXISTS) or select_when"
        elif feats["union"] and not feats["join"]:
            target = "union of programs -> `any` (fuses fully)"
        else:
            target = "per-file join -> capture-equality in program, or SQL join over t_export"
        rel.append({"rule": d["id"], "clause": c["id"], **feats, "becomes": target})
out["relational_clauses"] = len(rel)
out["relational_targets"] = Counter(r["becomes"] for r in rel)
out["relational_detail"] = rel

# fusion opportunity (from the compiled DAG)
dag = RUN["dag"]
out["dag"] = {k: dag[k] for k in ("ops", "expansions", "relations", "shared_relations", "recursive_dependent_relations", "memo_ids",
                                   "mask_scan_columns", "mask_all_columns", "max_stratum", "util_slots", "regexes", "patterns")}
out["relation_sites_expanded"] = dag["stats"]
out["strata_hist_queries"] = dag["strata_hist"]
out["mask_targets_per_stratum"] = dag["targets_per_stratum"]
tg = RUN["trie_groups"]
out["trie_groups"] = {"groups": len(tg), "K_top": tg[:12], "K>=16": sum(1 for _, k in tg if k >= 16),
                      "queries_in_K>=16": sum(k for _, k in tg if k >= 16), "unconstrained": RUN["unconstrained_queries"]}
bools = Counter()
for q in Q:
    bools.update(q["bools"])
out["bool_sites_expanded"] = dict(bools)
out["distinct_rule_digests"] = len({q["rule_digest"] for q in Q})
# relation targets shared across queries (static digest of relation bodies)
tgt = Counter()
for q in Q:
    for s in {tuple(x[:4]) for x in q["sites"]}:
        tgt[s] += 1
out["relation_targets_distinct_static"] = len(tgt)
out["relation_targets_shared_by>=2_queries"] = sum(1 for v in tgt.values() if v >= 2)
# candidate scope digests (for batch grouping)
scopes = Counter(json.dumps([r["include"], r["exclude"]]) for r in rules)
out["scope_digests"] = len(scopes)
out["largest_scope_groups"] = sorted(scopes.values(), reverse=True)[:8]
# per-query candidate concentration
qs = RUN["queries"]
tot = sum(q["sep_seconds"] for q in qs)
qs2 = sorted(qs, key=lambda q: -q["sep_seconds"])
acc, n80 = 0, 0
for q in qs2:
    acc += q["sep_seconds"]
    n80 += 1
    if acc >= 0.8 * tot:
        break
out["queries_for_80pct_sep_time"] = n80
out["top_queries"] = [{k: q[k] for k in ("rule", "clause", "role", "stratum", "root_kinds", "files", "candidates", "matches", "sep_seconds")} for q in qs2[:10]]
json.dump(out, open(os.path.join(OUT, "summary.json"), "w"), indent=1)
print(json.dumps({k: v for k, v in out.items() if k not in ("relational_detail", "top_queries")}, indent=1))
