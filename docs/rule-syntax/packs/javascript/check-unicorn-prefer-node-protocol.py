"""Checks that unicorn-prefer-node-protocol.modules.rr lists exactly the modules the policy JSON's regex names.
    python3 docs/rule-syntax/packs/javascript/check-unicorn-prefer-node-protocol.py"""
import os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
gen = __import__("gen-unicorn-prefer-node-protocol")

table = re.findall(r'bare_builtin\("([^"]+)"\)\.', open(os.path.join(gen.HERE,
        "unicorn-prefer-node-protocol.modules.rr")).read())
want = gen.names()
if sorted(table) != want:
    sys.exit(f"differs: table only {sorted(set(table) - set(want))}, JSON only {sorted(set(want) - set(table))}")
print(f"ok: {len(table)} modules")
