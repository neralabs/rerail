"""Checks ruff-ruf013-implicit-optional.stdlib.rr: it round-trips against Ruff's known_stdlib.rs, and its modules at
Python 3.10 are the JSON's `known-name` list.
    python3 docs/rule-syntax/packs/python/check-ruff-ruf013-implicit-optional.py
"""
import importlib.util, json, os, re
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gen", os.path.join(HERE, "gen-ruff-ruf013-implicit-optional.py"))
gen = importlib.util.module_from_spec(spec); spec.loader.exec_module(gen)
text = open(gen.OUT).read()
every = re.findall(r'stdlib_module\("([^"]+)"\)\.', text)
some = [(m, int(v)) for m, v in re.findall(r'stdlib_module_in\("([^"]+)", (\d+)\)\.', text)]
bad = 0
if (every, some) != gen.split(gen.table(open(gen.RS).read())):
    bad += 1
    print("known_stdlib.rs: DIFFERENT")
else:
    print(f"known_stdlib.rs: {len(every)} at every version, {len(some)} (module, version) rows, 0 differences")
at310 = set(every) | {m for m, v in some if v == 10}
want = set(gen.modules(json.load(open(gen.JSON))))
print(f"JSON (3.10): {len(want)} modules; {len(want - at310)} missing from the table, {len(at310 - want)} extra")
bad += want != at310
raise SystemExit(1 if bad else 0)
