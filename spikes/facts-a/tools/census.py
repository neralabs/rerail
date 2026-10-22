#!/usr/bin/env python3
"""Stage census for batch 6 (Spike A step 1).

Every relational input (AST evidence query), syntax spec, SQL stage, final query, forbid clause and
budget clause of the ten batch-6 policies is classified:
  a  AST-schema relation that becomes an extracted fact (node/child/field/ownership/descendant
     tables and the Python-derived relations function/class/call/argument/attribute/assignment/...)
  b  rule logic derivable from facts (joins, antijoins, recursion, aggregates, string predicates)
  c  bespoke: needs a fact not in schema v0 (decoded Python string values, f-string and annotation
     sub-grammars, Python splitlines over raw text)
Usage: census.py <policies dir> <out.md> <out.json>
"""
import json, re, sys, collections, pathlib

RULES = ["serverplan-bypass", "serverplan-steps", "serverplan-state-tripwires",
         "serverplan-dispatch-architecture", "serverplan-quality", "serverplan-target-architecture",
         "execution-authority-bypass", "impl-direct-execution", "raw-execute-regression",
         "legacy-execution-surfaces"]

# SQL stages that re-materialise the syntax tree or a Python-derived relation of it (category a).
SCHEMA_STAGES = {
    "syntax_rows", "nodes", "edges", "ownership", "descendants", "node_ordinals", "py_names",
    "identifier_nodes", "import_ranges", "import_identifiers", "functions", "classes", "methods",
    "module_functions", "attributes", "calls", "arguments", "keywords", "parameters", "assignments",
    "returns", "fields", "subscripts", "annotation_edges", "decorators", "statements", "frames",
    "parents", "scoped_nodes", "method_nodes", "method_descendants", "body_nodes", "paths",
    "strings", "comments", "bindings", "files", "branches", "scopes", "last_methods", "last_module_functions",
    "ordered_occurrences", "branch_flags", "members", "bases", "union_names", "symbols",
    "expression_edges", "callable_edges", "receiver_edges", "path_edges", "propagation",
}
# Stages whose rows need a fact outside schema v0 (category c).
BESPOKE_STAGES = {"string_values", "module_string_values", "builder_tokens"}
# Inputs whose AST query needs a sub-grammar or value decoding (category c).
BESPOKE_KEYS = {"value", "parse", "transform"}
SQL_CONSTRUCTS = [
    ("join", r"\bJOIN\b"), ("left join", r"\bLEFT JOIN\b"), ("exists / not exists (semi/anti join)", r"\bEXISTS\s*\("),
    ("not in / in (subquery)", r"\bIN\s*\(\s*SELECT"), ("union (set)", r"\bUNION\b(?!\s+ALL)"), ("union all", r"\bUNION ALL\b"),
    ("recursive cte", r"\bWITH RECURSIVE\b"), ("group by / aggregate", r"\bGROUP BY\b|\bcount\(|\bmin\(|\bmax\(|group_concat"),
    ("window row_number", r"row_number\(\)"), ("subquery (scalar or derived table)", r"\(SELECT (?!1 FROM)"), ("order by ... limit", r"\bLIMIT\b"),
    ("distinct", r"\bDISTINCT\b"), ("case", r"\bCASE\b"), ("regex fn (rx_*)", r"\brx_(matches|extract|replace)\("),
    ("string ops (|| substr instr trim lower upper replace length char like)", r"\|\||\bsubstr\(|\binstr\(|\btrim\(|\blower\(|\bupper\(|\breplace\(|\blength\(|\bchar\(|\bLIKE\b"),
    ("json_each / json_extract", r"json_each|json_extract|json_quote"), ("cast", r"\bCAST\("),
]

def keys_of(v, out):
    if isinstance(v, dict):
        for k, x in v.items():
            out.add(k); keys_of(x, out)
    elif isinstance(v, list):
        for x in v: keys_of(x, out)

def classify_input(name, ev):
    keys = set(); keys_of(ev["query"], keys)
    proj = ev.get("project", {})
    if keys & BESPOKE_KEYS:
        return "c", "needs " + "/".join(sorted(keys & BESPOKE_KEYS))
    node_dump = re.match(r"^(syntax_\d+|raw_\w+|nodes?)$", name) is not None
    if node_dump or ("kind" in proj and "text" in proj and len(proj) >= 3):
        return "a", "node dump (kind/regex/gate filter + field captures)"
    return "b", "structural predicate (has/inside/field/kind/regex)"

def main():
    root, out_md, out_json = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
    sys.path.insert(0, str(pathlib.Path(__file__).parent))
    from resolve import resolve
    rows, rules, constructs = [], [], collections.Counter()
    for rule in RULES:
        doc = resolve(json.load(open(root / f"data-layer-{rule}.json")))
        r = dict(rule=rule, clauses=0, relational=0, forbid=0, budget=0, inputs=0, stages=0, a=0, b=0, c=0)
        for clause in doc["clauses"]:
            if "define" in clause:
                continue
            r["clauses"] += 1
            cid = clause["id"]
            if "forbid" in clause:
                r["forbid"] += 1
                f = clause["forbid"]
                if "regex" in f:
                    lines = "lines" in f["regex"]
                    cat, why = ("c", "regex over raw text with Python splitlines") if lines else ("b", "regex over file text")
                else:
                    keys = set(); keys_of(f, keys)
                    cat, why = ("c", "needs " + "/".join(sorted(keys & BESPOKE_KEYS))) if keys & BESPOKE_KEYS else ("b", "AST pattern clause")
                rows.append(dict(rule=rule, clause=cid, unit="forbid", name=cid, cat=cat, why=why)); r[cat] += 1
                continue
            if "budget" in clause:
                r["budget"] += 1
                rows.append(dict(rule=rule, clause=cid, unit="budget", name=cid, cat="b", why="AST pattern + group count")); r["b"] += 1
                continue
            rel = clause["relational"]; r["relational"] += 1
            for name, inp in rel["inputs"].items():
                cat, why = classify_input(name, inp["evidence"])
                rows.append(dict(rule=rule, clause=cid, unit="input", name=name, cat=cat, why=why)); r[cat] += 1; r["inputs"] += 1
            if "syntax" in rel:
                rows.append(dict(rule=rule, clause=cid, unit="syntax", name=rel["syntax"]["name"], cat="a", why="Facts::build node/edge/ownership tables")); r["a"] += 1
            for st in rel.get("stages", []):
                base = re.sub(r"_\d+$", "", st["name"])
                sql = st.get("query", "")
                for label, pat in SQL_CONSTRUCTS:
                    if re.search(pat, re.sub(r"'(?:[^']|'')*'", "''", sql), re.I): constructs[label] += 1
                if "reachability" in st:
                    cat, why = "b", "reachability closure"
                elif base in BESPOKE_STAGES:
                    cat, why = "c", "decoded string values / module text"
                elif base in SCHEMA_STAGES:
                    cat, why = "a", "AST-schema relation rebuilt in SQL"
                else:
                    cat, why = "b", "rule logic"
                rows.append(dict(rule=rule, clause=cid, unit="stage", name=st["name"], cat=cat, why=why)); r[cat] += 1; r["stages"] += 1
            q = rel["query"]
            for label, pat in SQL_CONSTRUCTS:
                if re.search(pat, re.sub(r"'(?:[^']|'')*'", "''", q), re.I): constructs[label] += 1
            rows.append(dict(rule=rule, clause=cid, unit="final", name="query", cat="b", why="final projection/filter")); r["b"] += 1
        rules.append(r)
    json.dump(dict(rules=rules, units=rows, constructs=constructs), open(out_json, "w"), indent=1)
    with open(out_md, "w") as f:
        f.write("| rule | clauses | relational | forbid | budget | AST inputs | SQL stages | (a) schema | (b) logic | (c) bespoke |\n|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|\n")
        tot = collections.Counter()
        for r in rules:
            f.write(f"| `{r['rule']}` | {r['clauses']} | {r['relational']} | {r['forbid']} | {r['budget']} | {r['inputs']} | {r['stages']} | {r['a']} | {r['b']} | {r['c']} |\n")
            for k in ("clauses","relational","forbid","budget","inputs","stages","a","b","c"): tot[k] += r[k]
        f.write(f"| **total** | {tot['clauses']} | {tot['relational']} | {tot['forbid']} | {tot['budget']} | {tot['inputs']} | {tot['stages']} | {tot['a']} | {tot['b']} | {tot['c']} |\n\n")
        f.write("Bespoke (c) units:\n\n")
        for u in rows:
            if u["cat"] == "c": f.write(f"- `{u['rule']}` / `{u['clause']}` / {u['unit']} `{u['name']}`: {u['why']}\n")
        f.write("\nSQL constructs (number of stages + final queries using each):\n\n| construct | statements |\n|---|--:|\n")
        for k, v in constructs.most_common(): f.write(f"| {k} | {v} |\n")
main()
