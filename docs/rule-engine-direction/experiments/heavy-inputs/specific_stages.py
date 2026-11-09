"""Print only clause-specific stages (skipping the generator's common CST-rebuild stages) and the final query.
Usage: python3 -I specific_stages.py RULE_JSON CLAUSE_ID [maxchars]"""
import json, os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "fusion-model"))
import analyze
COMMON = {"nodes","edges","fields","functions","ownership","classes","methods","attributes","calls","arguments","keywords",
          "descendants","returns","assignments","parameters","nonempty_keywords","compiled","callable_edges","user_callable"}
path, cid = sys.argv[1], sys.argv[2]
mx = int(sys.argv[3]) if len(sys.argv) > 3 else 1500
raw = json.load(open(path)); _, opts = analyze.find_queries(raw); rule = analyze.resolve_options(raw, opts)
for c in rule["clauses"]:
    if c.get("id") != cid: continue
    rel = c["relational"]
    for s in rel.get("stages") or []:
        if s.get("name") in COMMON: continue
        q = s.get("query"); q = q if isinstance(q, str) else json.dumps(s)
        print(f"-- {s['name']}: {q[:mx]}\n")
    fq = rel.get("query"); fq = fq if isinstance(fq, str) else json.dumps(fq)
    print(f"-- FINAL: {fq[:mx*3]}")
