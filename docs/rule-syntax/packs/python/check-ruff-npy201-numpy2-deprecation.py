"""Checks that ruff-npy201-numpy2-deprecation.members.rr round-trips: rebuilding each clause's names, NEW
transform (the replacement's last segment), fallback exemption and message from the table gives the JSON's; and
the shared fallback list is the set of replacements."""
import importlib.util, json, os, re
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gen", os.path.join(HERE, "gen-ruff-npy201-numpy2-deprecation.py"))
gen = importlib.util.module_from_spec(spec); spec.loader.exec_module(gen)
policy = json.load(open(gen.JSON))
text = open(gen.OUT).read()
S = r'"((?:[^"\\]|\\.)*)"'
adv = [(json.loads(f'"{g}"'), json.loads(f'"{m}"')) for g, m in re.findall(rf"npy201_advice\({S}, {S}\)\.", text)]
mem = re.findall(r'npy201_member\("([^"]+)", "([^"]+)"\)\.', text)
rep = dict(re.findall(r'npy_replacement\("([^"]+)", "([^"]+)"\)\.', text))
bad = 0
clauses = {c["id"]: c for c in policy["clauses"]}
for g, m in adv:
    c = clauses[g]; rule = c["forbid"]["ast"]["rule"]
    qs = [q for q, h in mem if h == g]
    news = {rep.get(q, "") for q in qs}
    new = news.pop() if len(news) == 1 else None
    tr = c["forbid"]["ast"].get("transform", {})
    ok = (qs == rule["resolves"]["import"] and m == c["violation"]["message"] and ("not" in rule) == bool(new)
          and (tr.get("NEW", {}).get("replace", {}).get("by") == new.rsplit(".", 1)[-1] if new else not tr))
    bad += not ok
fallback = clauses["shared"]["define"]["utils"]["fallback"]["any"][0]["inside"]["has"]["has"]["resolves"]["import"]
bad += sorted(set(rep.values())) != sorted(fallback)
bad += len(adv) != len(gen.groups(policy))
print(f"round trip: {len(mem)} members in {len(adv)} groups, {bad} differences")
raise SystemExit(1 if bad else 0)
