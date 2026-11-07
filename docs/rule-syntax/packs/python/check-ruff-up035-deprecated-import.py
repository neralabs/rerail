"""Checks that ruff-up035-deprecated-import.moves.rr says what the JSON says: every clause's names, versions and
message are in the table, and the table holds nothing else.
    python3 docs/rule-syntax/packs/python/check-ruff-up035-deprecated-import.py     (prints ok, or the differences)"""
import json, os, re, sys
sys.dont_write_bytecode = True

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
gen = __import__("gen-ruff-up035-deprecated-import")

text = open(gen.OUT).read()
moved = set(re.findall(r'^moved\("([^"]*)", "([^"]*)", "([^"]*)"\)\.$', text, re.M))
says = {c: (int(lo), int(hi), json.loads(m)) for c, lo, hi, m in
        re.findall(r'^move_says\("([^"]*)", (\d+), (\d+), ("(?:[^"\\]|\\.)*")\)\.$', text, re.M)}
bad = []
claimed = set()
for cid, mod, names, lo, hi, msg in gen.clauses(json.load(open(gen.JSON))):
    if says.get(cid) != (lo, hi, msg):
        bad.append(f"clause {cid}: {says.get(cid)} != {(lo, hi, msg)}")
    for x in names:
        # the name is reported for this clause's whole range: by this clause, or one of the same module that
        # starts no later and ends at the same version
        owners = [c for m, y, c in moved if m == mod and y == x and says[c][1] == hi and says[c][0] <= lo]
        if not owners:
            bad.append(f"{mod}.{x} ({cid}) missing")
        claimed.update((mod, x, c) for c in owners)
for row in sorted(moved - claimed):
    bad.append(f"extra row {row}")
print("\n".join(bad) if bad else "ok")
sys.exit(1 if bad else 0)
