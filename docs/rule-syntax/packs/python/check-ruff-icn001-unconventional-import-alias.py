"""Checks that ruff-icn001-unconventional-import-alias.aliases.rr round-trips: rebuilding the JSON's module regexes
from the table gives the JSON's transforms, in order."""
import importlib.util, json, os, re
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gen", os.path.join(HERE, "gen-ruff-icn001-unconventional-import-alias.py"))
gen = importlib.util.module_from_spec(spec); spec.loader.exec_module(gen)
policy = json.load(open(gen.JSON))
tr = policy["clauses"][0]["forbid"]["ast"]["transform"]
want = [(tr[k]["replace"]["replace"], tr[k]["replace"]["by"]) for k in sorted((k for k in tr if k != "E"), key=lambda k: int(k[1:])) + ["E"]]
got = [("^" + m.replace(".", gen.DOT) + "$", a) for m, a in re.findall(r'icn_alias\("([^"]+)", "([^"]+)"\)\.', open(gen.OUT).read())]
# The aliases the third clause accepts are the same set.
alts = re.fullmatch(r"\^\(\?:(.*)\)\$", policy["clauses"][2]["forbid"]["ast"]["rule"]["any"][2]["not"]["regex"]).group(1).split("|")
ok = got == want and alts == [a for _, a in got]
print(f"round trip: {len(got)} aliases, {'0 differences' if ok else 'DIFFERENT'}")
raise SystemExit(0 if ok else 1)
