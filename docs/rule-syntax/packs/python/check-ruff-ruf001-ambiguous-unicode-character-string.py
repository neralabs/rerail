"""Round-trips ruff-ruf001-ambiguous-unicode-character-string.confusables.rr against the JSON's character classes
(the three RUF001-003 JSONs share them), and compares both with Ruff's own table (confusables.rs) when it is
present: Ruff reports a look-alike only when its representant is ASCII.
    python3 docs/rule-syntax/packs/python/check-ruff-ruf001-ambiguous-unicode-character-string.py
"""
import importlib.util, json, os, re, sys
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gen", os.path.join(HERE, "gen-ruff-ruf001-ambiguous-unicode-character-string.py"))
gen = importlib.util.module_from_spec(spec); spec.loader.exec_module(gen)
RULES = os.path.join(HERE, "..", "..", "..", "..", "packs", "python", "rules")
RS = ("/Users/jeremy/orca/workspaces/rerail/wolfeel/.rerail/port-readiness/upstream/ruff/"
      "0be08a206f9c3180afd3e93bcc792ed5cb1f4db1/crates/ruff_linter/src/rules/ruff/rules/confusables.rs")

text = open(gen.OUT, encoding="utf-8").read()
def from_rr(head):
    cps = set()
    for line in text.splitlines():
        if line.startswith(head) and "match_all(\"(?<ch>[" in line:
            body = line[line.index("(?<ch>[") + 6:line.rindex("])\"") + 1].replace("\\\\", "\\")
            cps |= set(gen.parse_class(body))
    return cps
rr = (from_rr("confusable_char(t, ch) :- confusable_text"), from_rr("confusable_letter("))
bad = 0
for n in ["001-ambiguous-unicode-character-string", "002-ambiguous-unicode-character-docstring",
          "003-ambiguous-unicode-character-comment"]:
    js = tuple(set(c) for c in gen.classes(json.load(open(os.path.join(RULES, f"ruff-ruf{n}.json")))))
    for name, a, b in [("symbols", rr[0], js[0]), ("letters", rr[1], js[1])]:
        if a != b:
            bad += 1
            print(f"RUF{n[:3]} {name}: {len(a - b)} only in .rr, {len(b - a)} only in JSON")
print(f"round trip: {len(rr[0])} symbols, {len(rr[1])} letters, {bad} differences")
if os.path.exists(RS):
    pairs = re.findall(r"^\s*(0x[0-9A-Fa-f]+|[0-9_]+)u?(?:32)?(?:_u32)? => '(.+)',$", open(RS, encoding="utf-8").read(), re.M)
    ascii_rep = {int(k.replace("_", ""), 0) for k, v in pairs if len(v) == 1 and ord(v) < 128 or v in ("\\\\", "\\'")}
    both = rr[0] | rr[1]
    print(f"confusables.rs: {len(pairs)} entries, {len(ascii_rep)} with an ASCII representant; "
          f"{len(ascii_rep - both)} missing from the table, {len(both - ascii_rep)} extra")
sys.exit(1 if bad else 0)
