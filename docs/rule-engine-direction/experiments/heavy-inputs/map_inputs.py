"""Map fusion-model per-query timings onto named relational inputs, and report how each input is consumed.

Usage: python3 -I map_inputs.py RULES_DIR FUSION_OUT OUT_JSON
"""
import json, os, re, sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "fusion-model"))
import analyze  # noqa: E402

rules_dir, fout, out_path = sys.argv[1:4]
run = json.load(open(os.path.join(fout, "run_full.json")))
qfull = json.load(open(os.path.join(fout, "queries_full.json")))
active = [q for q in qfull if q.get("active")]
assert len(active) == len(run["queries"]), (len(active), len(run["queries"]))

# resolved rules
resolved = {}
for fn in sorted(os.listdir(rules_dir)):
    if not fn.endswith(".json"):
        continue
    raw = json.load(open(os.path.join(rules_dir, fn)))
    _, opts = analyze.find_queries(raw)
    resolved[raw["id"]] = (analyze.resolve_options(raw, opts), opts)


def find_input(rule_id, clause_id, query):
    rule, _ = resolved[rule_id]
    for c in rule["clauses"]:
        if c.get("id") != clause_id or "relational" not in c:
            continue
        rel = c["relational"]
        for name, inp in (rel.get("inputs") or {}).items():
            ev = inp.get("evidence", inp) if isinstance(inp, dict) else {}
            qq = (ev.get("query") or {})
            qq = qq.get("ast", qq) if isinstance(qq, dict) else qq
            if isinstance(qq, dict) and qq.get("rule") == query.get("rule"):
                return name, ev, rel
    return None, None, None


def stage_texts(rel):
    out = []
    for s in rel.get("stages") or []:
        q = s.get("query")
        out.append((s.get("name"), q if isinstance(q, str) else json.dumps(q)))
    fq = rel.get("query")
    out.append(("__final__", fq if isinstance(fq, str) else json.dumps(fq)))
    return out


rows = []
for q, t in zip(active, run["queries"]):
    assert q["rule"] == t["rule"] and q["clause"] == t["clause"]
    if q["role"] != "relational":
        continue
    name, inp, rel = find_input(q["rule"], q["clause"], q["query"])
    proj = inp.get("project") if isinstance(inp, dict) else None
    users = []
    if rel is not None and name:
        pat = re.compile(r"\b%s\b" % re.escape(name))
        for sn, txt in stage_texts(rel):
            if txt and pat.search(txt):
                users.append(sn)
    rows.append(dict(rule=q["rule"], clause=q["clause"], input=name, sep_seconds=t["sep_seconds"],
                     candidates=t["candidates"], matches=t["matches"], files=t["files"],
                     root_kinds=t["root_kinds"], stratum=t["stratum"],
                     project_cols=(list(proj.keys()) if isinstance(proj, dict) else proj),
                     used_by=users))

rows.sort(key=lambda r: -r["sep_seconds"])
json.dump(rows, open(out_path, "w"), indent=1)
tot = sum(r["sep_seconds"] for r in rows)
alltot = sum(t["sep_seconds"] for t in run["queries"])
print(f"relational inputs: {len(rows)}  sep total {tot:.1f}s of {alltot:.1f}s ({100*tot/alltot:.0f}%)")
print(f"exported rows (matches) total: {sum(r['matches'] for r in rows):,}")
for r in rows[:45]:
    print(f"{r['sep_seconds']:6.2f}s  {r['rule'][11:]:28s} {r['clause']:28s} {str(r['input']):30s} cand={r['candidates']:>9,} rows={r['matches']:>9,} cols={len(r['project_cols'] or [])} used={r['used_by'][:6]}")
