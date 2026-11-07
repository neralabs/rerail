"""Checks that eslint-no-irregular-whitespace.chars.rr says what the policy JSON says: the same code points in the
same two classes. python3 docs/rule-syntax/packs/javascript/check-eslint-no-irregular-whitespace.py"""
import os, re, sys
sys.dont_write_bytecode = True  # importing the generator leaves no __pycache__ beside the rules
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
gen = __import__("gen-eslint-no-irregular-whitespace")

table = []
for m in re.finditer(r'^irregular\((\d+), "(.*)", "(\w+)"\)\.  // U\+[0-9A-F]+$', open(os.path.join(gen.HERE,
        "eslint-no-irregular-whitespace.chars.rr"), encoding="utf-8").read(), re.M):
    cp, fr, what = int(m[1]), m[2], m[3]
    ch = chr(int(fr[4:], 16)) if fr.startswith("\\\\x") else fr
    assert ord(ch) == cp, (cp, fr)
    table.append((cp, what))
want = gen.rows()
if sorted(table) != sorted(want):
    sys.exit(f"differs: table only {sorted(set(table) - set(want))}, JSON only {sorted(set(want) - set(table))}")
# The rule writes the two classes out as one regex literal (match_all needs its group name there): read the
# code points of each bracket back, ranges expanded, and compare them with the table.
rule = open(os.path.join(gen.HERE, "eslint-no-irregular-whitespace.rr"), encoding="utf-8").read()
m = re.search(r'"\(\?<W>\[(.*?)\]\+\|\[(.*?)\]\)"', rule)
assert m, "the rule's match_all regex was not found"
def cps(cls):
    out = set()
    for a, b in re.findall(r"\\\\x\{([0-9A-F]+)\}(?:-\\\\x\{([0-9A-F]+)\})?", cls):
        out |= set(range(int(a, 16), int(b or a, 16) + 1))
    return out
got = {(c, "run") for c in cps(m[1])} | {(c, "line") for c in cps(m[2])}
if got != set(table):
    sys.exit(f"rule regex differs: rule only {sorted(got - set(table))}, table only {sorted(set(table) - got)}")
print(f"ok: {len(table)} code points, in the table and in the rule's regex")
