"""Removes the imports the language-pack rewrite left unused: rustc's own suggestions (cargo check JSON on stdin),
for unused-import warnings naming only `with_adapter` and the adapter traits. Other warnings are left alone.

usage: cargo check --message-format json | python3 unused_adapter_imports.py [--write]
"""
import json, os, re, sys
NAMES = {'with_adapter', 'Adapter', 'SyntaxRules', 'ScopeRules', 'ValueRules', 'ArgumentRules', 'CallRules',
         'FormatArgRules'}
edits = {}
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    d = m.get('message') if m.get('reason') == 'compiler-message' else None
    if not d or not (d.get('code') or {}).get('code') == 'unused_imports':
        continue
    named = {n.split('::')[-1] for n in re.findall(r'`([^`]+)`', d['message'])}
    if not named or not named <= NAMES:
        continue
    for c in d['children']:
        for sp in c['spans']:
            if sp.get('suggestion_applicability') == 'MachineApplicable' and sp.get('suggested_replacement') is not None:
                edits.setdefault(sp['file_name'], set()).add((sp['byte_start'], sp['byte_end'], sp['suggested_replacement']))
n = 0
for f, es in edits.items():
    es = sorted(es)
    for a, b in zip(es, es[1:]):
        assert a[1] <= b[0], (f, a, b)
    n += len(es)
    if '--write' in sys.argv:
        s = open(f, 'rb').read()
        for start, end, rep in reversed(es):
            s = s[:start] + rep.encode() + s[end:]
        open(f, 'wb').write(s)
print(f'{n} removals in {len(edits)} files')
