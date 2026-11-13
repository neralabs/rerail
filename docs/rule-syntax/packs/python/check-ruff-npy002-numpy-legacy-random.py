"""Checks that ruff-npy002-numpy-legacy-random.methods.rr round-trips: each group's functions and message are the
JSON clause's, and the string-annotation clause lists every function once."""
import importlib.util, json, os, re
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gen", os.path.join(HERE, "gen-ruff-npy002-numpy-legacy-random.py"))
gen = importlib.util.module_from_spec(spec); spec.loader.exec_module(gen)
policy = json.load(open(gen.JSON))
text = open(gen.OUT).read()
adv = [(json.loads(f'"{g}"'), json.loads(f'"{m}"')) for g, m in re.findall(r'npy002_advice\("((?:[^"\\]|\\.)*)", "((?:[^"\\]|\\.)*)"\)\.', text)]
meth = re.findall(r'npy002_method\("([^"]+)", "([^"]+)"\)\.', text)
got = [(g, [q for q, h in meth if h == g], m) for g, m in adv]
want = gen.groups(policy)
every = [c for c in policy["clauses"] if c["id"] == "string-annotation"][0]["forbid"]["ast"]["utils"]["reference"]["all"][0]["resolves"]["import"]
ok = got == want and sorted(every) == sorted(q for q, _ in meth)
print(f"round trip: {len(meth)} functions in {len(adv)} groups, {'0 differences' if ok else 'DIFFERENT'}")
raise SystemExit(0 if ok else 1)
