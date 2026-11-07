"""Per relational clause: inputs, stages, partitionBy, features, measured input cost/rows. Usage: python3 -I clauses.py RULES FUSION_OUT INPUTS_JSON"""
import json, os, sys, collections
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "fusion-model"))
import analyze
rules_dir, fout, inj = sys.argv[1:4]
summ = json.load(open(os.path.join(fout, "summary.json")))
det = {(d["rule"], d["clause"]): d for d in summ["relational_detail"]}
rows = json.load(open(inj))
agg = collections.defaultdict(lambda: [0.0, 0, 0])
for r in rows:
    a = agg[(r["rule"], r["clause"])]; a[0] += r["sep_seconds"]; a[1] += r["matches"]; a[2] += r["matches"] * len(r["project_cols"] or [])
for fn in sorted(os.listdir(rules_dir)):
    raw = json.load(open(os.path.join(rules_dir, fn))); _, opts = analyze.find_queries(raw); rule = analyze.resolve_options(raw, opts)
    for c in rule["clauses"]:
        if "relational" not in c: continue
        rel = c["relational"]; k = (rule["id"], c["id"]); d = det.get(k, {})
        st = rel.get("stages") or []
        allsql = " ".join((s.get("query") if isinstance(s.get("query"), str) else json.dumps(s)) for s in st) + " " + (rel.get("query") if isinstance(rel.get("query"), str) else "")
        a = agg.get(k, [0, 0, 0])
        print(f"{rule['id'][11:]:30s} {c['id']:32s} in={len(rel.get('inputs') or {}):2d} st={len(st):2d} part={rel.get('partitionBy','-'):5s} syn={'Y' if 'syntax' in rel else '-'} ref={'Y' if 'references' in rel else '-'} rec={allsql.count('RECURSIVE')+sum(1 for s in st if 'reachability' in s):2d} reach={'Y' if any('reachability' in s for s in st) else '-'} xf={'Y' if d.get('cross_file') else '-'} sep={a[0]:6.2f}s rows={a[1]:>9,} cells={a[2]:>11,} sqlKB={len(allsql)//1024}")
