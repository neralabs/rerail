"""Dump a relational clause: inputs (rule + project), utilities referenced, stages (name, size, SQL head).

Usage: python3 -I dump_clause.py RULE_JSON CLAUSE_ID [--full-sql] [--stage NAME]
"""
import json, os, sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "fusion-model"))
import analyze  # noqa: E402

path, cid = sys.argv[1], sys.argv[2]
full = "--full-sql" in sys.argv
only = sys.argv[sys.argv.index("--stage") + 1] if "--stage" in sys.argv else None
raw = json.load(open(path))
_, opts = analyze.find_queries(raw)
rule = analyze.resolve_options(raw, opts)
print("options:", opts)
utils = {}
for c in rule["clauses"]:
    if "define" in c:
        for k, v in (c["define"].get("utils") or c["define"].get("rules") or {}).items():
            utils[k] = v
for c in rule["clauses"]:
    if c.get("id") != cid:
        continue
    rel = c["relational"]
    print("keys:", list(rel.keys()))
    for k in rel:
        if k not in ("inputs", "stages", "query"):
            print(f"  {k}: {json.dumps(rel[k])[:600]}")
    if only is None:
        for name, inp in (rel.get("inputs") or {}).items():
            ev = inp.get("evidence", inp)
            q = ev.get("query", {})
            print(f"\n== input {name} as={ev.get('as')}")
            print("  rule:", json.dumps(q)[:1400])
            print("  project:", json.dumps(ev.get("project"))[:1200])
            for k in ev:
                if k not in ("query", "project", "as"):
                    print(f"  {k}: {json.dumps(ev[k])[:300]}")
    for s in rel.get("stages") or []:
        if only and s.get("name") != only:
            continue
        q = s.get("query")
        q = q if isinstance(q, str) else json.dumps(q)
        extra = {k: v for k, v in s.items() if k not in ("name", "query")}
        print(f"\n-- stage {s.get('name')} ({len(q)} chars) {json.dumps(extra)[:300]}")
        print(q if full else q[:700])
    if only is None:
        fq = rel.get("query")
        fq = fq if isinstance(fq, str) else json.dumps(fq)
        print(f"\n-- final ({len(fq)} chars)\n{fq if full else fq[:1500]}")
