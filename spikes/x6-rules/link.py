#!/usr/bin/env python3
"""Reader binding for the declared-facts pack, by hand (no compiler work): binds rules that read
facts by name into a `rerail.policy/v1` document today's compiler accepts.

- A per-content fact (`over: content`, `define.match`) becomes a matcher input under its own name;
  the compiler shares an input across the clauses of one document by its evidence text, so each
  per-content fact is computed once per content for every reader in the document.
- A membership fact (`over: membership`, `define.sql`) becomes a stage under its own name, placed
  before its readers in declaration order.
- `--mode separate`: one clause per rule; membership facts are repeated in every clause that reads
  them (today's one-plan-per-clause lowering computes them once per reader).
- `--mode merged`: one clause holding every rule; each membership fact appears once, each rule's
  stages are prefixed `<rule>__`, and the clause's query is the union of the rules' findings
  (`rule`, `path`, `start_byte`, `end_byte`, `detail`). This is what a compiler with one plan graph
  across clauses (CSE by digest) would build; the per-rule outputs are the `<rule>__findings` stages.
- Scopes: a rule's `scope` (else the facts files'). Merged rules with different scopes share the
  union (`merged_scope`); a rule whose scope does not provably cover it reads every per-content fact
  through a `<rule>__scoped_<fact>` path filter, and may not read membership facts (they would be
  computed over the wider file set). Today's per-clause scope gives this for free.

usage: link.py --facts F.json[,F2.json] --rules R1.json,R2.json --mode separate|merged --out DOC.json
       [--id ID]"""
import argparse
import json
import re
from pathlib import Path

FACT_RE = re.compile(r"\b((?:py|proj|sel)_[a-z0-9_]+)\b")


def texts(q):
    """The SQL texts of a query: a string, an option switch (`cases`, `default`) or an option
    template (`template`)."""
    if isinstance(q, dict):
        if "template" in q:
            return [q["template"]]
        return [t for v in [*q["cases"].values(), *([q["default"]] if "default" in q else [])] for t in texts(v)]
    return [q]


def mapq(q, f):
    if isinstance(q, dict):
        if "template" in q:
            return {**q, "template": f(q["template"])}
        return {**q, "cases": {k: mapq(v, f) for k, v in q["cases"].items()}, **({"default": mapq(q["default"], f)} if "default" in q else {})}
    return f(q)


def stage_texts(s):
    """The SQL texts of a stage: its query, or a `reachability` stage's roots and edges."""
    if "reachability" in s:
        return [s["reachability"][k] for k in ("roots", "edges") if k in s["reachability"]]
    return texts(s["query"])


def load_facts(paths):
    facts, define, scope = {}, {"options": {}, "language": "python", "utils": {}}, None
    for p in paths:
        d = json.loads(Path(p).read_text())
        for k, v in d.get("define", {}).get("options", {}).items():
            assert define["options"].get(k, v) == v, f"option {k} declared twice, differently"
            define["options"][k] = v
        for k, v in d.get("define", {}).get("utils", {}).items():
            assert define["utils"].get(k, v) == v, f"util {k} declared twice, differently"
            define["utils"][k] = v
        if d.get("scope"):
            assert scope in (None, d["scope"]), "two scopes"
            scope = d["scope"]
        for k, v in d["facts"].items():
            assert k not in facts, f"fact {k} declared twice"
            facts[k] = v
    return facts, define, scope


def reads(rule, facts):
    """The facts a rule reads, transitively, in declaration order."""
    seen, todo = set(), []
    for s in rule["stages"]:
        todo += [t for q in stage_texts(s) for t in FACT_RE.findall(q)]
    todo += [t for q in texts(rule["query"]) for t in FACT_RE.findall(q)]
    while todo:
        f = todo.pop()
        if f in seen or f not in facts:
            assert f in facts or f in rule.get("inputs", {}), f"{rule['id']}: unknown fact {f}"
            continue
        seen.add(f)
        if facts[f]["over"] == "membership":
            todo += [t for q in texts(facts[f]["define"]["sql"]) for t in FACT_RE.findall(q)]
    return [f for f in facts if f in seen]


def bind(rules, facts):
    inputs, stages = {}, []
    needed = []
    for r in rules:
        for f in reads(r, facts):
            if f not in needed:
                needed.append(f)
    for f in [f for f in facts if f in needed]:
        d = facts[f]
        if d["over"] == "content":
            inputs[f] = {"evidence": d["define"]["match"]}
        else:
            stages.append({"name": f, "query": d["define"]["sql"], **({"indexes": d["indexes"]} if d.get("indexes") else {})})
    return inputs, stages


def merged_scope(scopes):
    """The scope of a document holding rules with these scopes: every include, the excludes all
    share (a superset of their union; each rule narrower than it filters, `scope_filter`)."""
    inc = [g for s in scopes for g in s["include"]]
    exc = [g for g in scopes[0].get("exclude", []) if all(g in s.get("exclude", []) for s in scopes)]
    return {"include": list(dict.fromkeys(inc)), "exclude": exc}


def glob_re(glob):
    """globset with literal_separator (`facts/src/policy.rs` `Scope`): `**/` any directories,
    `/**` anything below, `*` within one segment (as `facts-b/scripts/linker_check.py`)."""
    out = re.escape(glob).replace(r"\*\*/", "%DIRS%").replace(r"/\*\*", "%BELOW%").replace(r"\*", "[^/]*")
    return out.replace("%DIRS%", "(?:.*/)?").replace("%BELOW%", "/.*")


def covers(rule, doc):
    """Sound, incomplete: every path in scope `doc` is in scope `rule`. Each rule exclude is a doc
    exclude, and each doc include is a rule include or lies under a rule include `X/**/*.py`
    (a literal `X/` prefix and a last segment `*.py`)."""
    def under(g, h):
        x = h[:-len("/**/*.py")] if h.endswith("/**/*.py") else None
        return x is not None and not re.search(r"[*?\[{]", x) and g.startswith(x + "/") and g.endswith("/*.py")
    return all(e in doc.get("exclude", []) for e in rule.get("exclude", [])) and \
        all(g in rule["include"] or any(under(g, h) for h in rule["include"]) for g in doc["include"])


def scope_filter(rule, doc):
    """The SQL path filter selecting scope `rule` inside scope `doc`, or None when `rule` covers it."""
    if rule is None or rule == doc or covers(rule, doc):
        return None
    inc = "|".join(glob_re(g) for g in rule["include"])
    exc = "|".join(glob_re(g) for g in rule.get("exclude", []))
    return f"rx_matches(__path,'^(?:{inc})$')" + (f" AND NOT rx_matches(__path,'^(?:{exc})$')" if exc else "")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--facts", required=True)
    ap.add_argument("--rules", required=True)
    ap.add_argument("--mode", choices=["separate", "merged"], required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--id", default="x6-pack")
    a = ap.parse_args()
    facts, define, scope = load_facts(a.facts.split(","))
    rules = [json.loads(Path(p).read_text()) for p in a.rules.split(",")]
    for r in rules:
        define["options"].update({k: v for k, v in r.get("options", {}).items()})
        define["utils"].update({k: v for k, v in r.get("utils", {}).items()})
    # A rule may carry its own file scope (the document's it came from), else the facts files'.
    # `separate`: all rules linked into one document must agree, and it replaces the facts files'
    # scope. `merged`: the document's scope is their union (`merged_scope`), and a rule whose own
    # scope does not provably cover it reads its per-content facts through a scope filter.
    rule_scopes = [json.dumps(r["scope"], sort_keys=True) for r in rules if "scope" in r]
    facts_scope = scope
    if rule_scopes and a.mode == "separate":
        assert len(set(rule_scopes)) == 1 and len(rule_scopes) == len(rules), "rules with different scopes in one document"
        scope = rules[0]["scope"]
    elif rule_scopes:
        scope = merged_scope([r.get("scope", facts_scope) for r in rules])
    doc = {"format": "rerail.policy/v1", "id": a.id, "revision": 1, "mode": "log",
           "description": "x6-rules: declared-facts pack (disposable exploration).", "scope": scope,
           "clauses": [{"id": "shared", "define": define}]}
    passthrough = lambda r: {k: v for k, v in r.items() if k not in ("id", "alias", "violation", "inputs", "stages", "query", "options", "utils", "findings", "scope")}
    if a.mode == "separate":
        for r in rules:
            inputs, stages = bind([r], facts)
            for k, v in r.get("inputs", {}).items():
                assert k not in inputs
                inputs[k] = v
            rel = {"inputs": inputs, "stages": stages + r["stages"], "query": r["query"], **passthrough(r)}
            doc["clauses"].append({"id": r["id"], "relational": rel, "violation": r["violation"]})
    else:
        inputs, stages = bind(rules, facts)
        rel_extra, union = {}, []
        for r in rules:
            p = r.get("alias", r["id"]) + "__"
            own = {s["name"] for s in r["stages"]} | set(r.get("inputs", {}))
            # A rule narrower than the document reads its per-content facts and private inputs
            # through `<alias>__scoped_<table>` (its own scope as a path filter).
            scoped = {}
            pred = scope_filter(r.get("scope", facts_scope), scope)
            if pred:
                rf = reads(r, facts)
                assert all(facts[f]["over"] == "content" for f in rf), f"{r['id']}: reads membership facts over a scope narrower than the document's"
                for t in [*rf, *r.get("inputs", {})]:
                    scoped[t] = f"{p}scoped_{t}"
                    stages.append({"name": scoped[t], "query": f"SELECT * FROM {p + t if t in own else t} WHERE {pred}"})
            ren = lambda q: re.sub(r"\b(FROM|JOIN)(\s+)([a-z0-9_]+)\b", lambda m: m[1] + m[2] + scoped.get(m[3], p + m[3] if m[3] in own else m[3]), q)
            for k, v in r.get("inputs", {}).items():
                inputs[p + k] = v
            for s in r["stages"]:
                if "reachability" in s:  # `roots`/`edges` SQL (and `syntax`, a table name)
                    reach = {k: (p + v if k == "syntax" and v in own else ren(v) if k in ("roots", "edges") else v) for k, v in s["reachability"].items()}
                    stages.append({**s, "name": p + s["name"], "reachability": reach})
                    continue
                stages.append({**s, "name": p + s["name"], "query": mapq(s["query"], ren)})
            stages.append({"name": p + "findings", "query": mapq(r["query"], ren)})
            # The rule's findings, in the union's shape; `detail` keeps the rule's own columns.
            union.append(f"SELECT '{r['id']}' AS rule,path,start_byte,end_byte,{r['findings']['detail']} AS detail FROM {p}findings")
            for k, v in passthrough(r).items():
                if k in ("key", "code"):
                    continue
                assert rel_extra.get(k, v) == v, f"{k} differs between merged rules"
                rel_extra[k] = v
        rel = {"inputs": inputs, "stages": stages, "query": "\n UNION ALL ".join(union), "key": ["rule", "detail"], **rel_extra}
        doc["clauses"].append({"id": "pack", "relational": rel, "violation": {"code": "x6_pack", "message": "x6 pack finding", "anchor": "witness.span"}})
    Path(a.out).write_text(json.dumps(doc, indent=1) + "\n")


if __name__ == "__main__":
    main()
