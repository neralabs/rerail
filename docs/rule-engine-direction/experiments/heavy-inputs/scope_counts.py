"""Kind counts per rule scope (from rule-profiles files.tsv). Usage: python3 -I scope_counts.py RULES FILES_TSV rule-id..."""
import json, os, sys, csv
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "fusion-model"))
import analyze
rules_dir, tsv = sys.argv[1], sys.argv[2]
rows = list(csv.DictReader(open(tsv), delimiter="\t"))
cols = ["nodes", "call", "function_definition", "class_definition", "attribute", "assignment", "string", "identifier", "import_from_statement", "return_statement", "raise_statement"]
for rid in sys.argv[3:]:
    raw = json.load(open(os.path.join(rules_dir, rid + ".json")))
    sc = raw.get("scope") or {}
    inc = [analyze.glob_re(g) for g in sc.get("include", ["**"])]; exc = [analyze.glob_re(g) for g in sc.get("exclude", [])]
    sel = [r for r in rows if any(i.match(r["path"]) for i in inc) and not any(e.match(r["path"]) for e in exc)]
    tot = {c: sum(int(r[c]) for r in sel) for c in cols}
    print(rid[11:], "files", len(sel), " ".join(f"{c}={tot[c]:,}" for c in cols))
