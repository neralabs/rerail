"""Compare native survey inventory with the pinned Python scanner, offline only.

This is a producer-field contract, not full dead-code migration acceptance.
The native command reads a prior recording and never imports the source scanner.
"""
import argparse
import ast
import importlib
import json
from pathlib import Path
import sys

from migration import PIN, ROOT, git, sha, write_json


CASES = [
    ('empty', {'empty.py': ''}),
    ('definitions', {'impl/a.py': 'def one(): pass\nasync def two(): pass\nclass C:\n def method(self): pass\n async def amethod(self): pass\n'}),
    ('nested-functions', {'a.py': 'def outer():\n def inner(): pass\n class Hidden:\n  def omitted(self): pass\n return inner\n'}),
    ('nested-classes', {'a.py': 'class Outer:\n class Inner:\n  def method(self): pass\n def after(self): pass\n'}),
    ('conditional-definitions', {'a.py': 'if flag:\n def f(): pass\nelse:\n def f(): pass\nclass C:\n if flag:\n  def method(self): pass\n'}),
    ('decorated-order', {'a.py': '@decorate\ndef f(): pass\n@other\nclass C:\n @property\n def field(self): pass\n'}),
    ('unicode', {'a.py': 'class K:\n def ﬁeld(self): pass\ndef K(): pass\n'}),
    ('package-paths', {'pkg/__init__.py': 'class C: pass\n', 'pkg/child.py': 'def f(): pass\n', '__init__.py': 'def root(): pass\n'}),
    ('plain-imports', {'a.py': 'import x.y, first as second\nfrom lib import C, value as alias\nfrom lib import *\n'}),
    ('import-overwrite', {'a.py': 'from first import x\ndef f():\n from second import x\n import nested.module\nclass C:\n import final as nested\n'}),
    ('relative-imports', {'pkg/sub/a.py': 'from . import one\nfrom ..base import Two as Alias\nfrom ... import three\nfrom ....b import Four\n', 'pkg/sub/__init__.py': 'from . import one\nfrom ..base import Two\n', 'a.py': 'from . import skip\nfrom ..base import X\n'}),
    ('duplicate-aliases', {'a.py': 'from first import a as x, b as x\nimport a as y, b as y\nfrom third import x\n'}),
    ('future-imports', {'a.py': 'from __future__ import annotations\ndef f(): pass\n'}),
]


def source_rows(source, files):
    sys.path[:0] = [str(source), str(source / 'scripts')]
    model = importlib.import_module('lint_lib.symbol_graph')
    rows = []
    for path, text in files.items():
        module, info, symbols = model._scan_symbol_tree(Path('/survey'), Path('/survey') / path, ast.parse(text))
        rows.append(dict(kind='module', path=path, module=module))
        for i, s in enumerate(symbols):
            rows.append(dict(kind='symbol', path=path, module=module, ordinal=i, sid=s.symbol_id,
                             fq=s.fqname, name=s.name, qual=s.qualname, symbol_kind=s.kind,
                             line=s.line, owner=s.class_name or '', async_flag=int(s.is_async)))
        for name, ids in info.defined_names.items():
            for i, sid in enumerate(ids):
                rows.append(dict(kind='defined', path=path, module=module, name=name, sid=sid, ordinal=i))
        for kind, values in [('imported', info.imported_symbols), ('import_module', info.imported_modules)]:
            rows.extend(dict(kind=kind, path=path, module=module, name=k, target=v) for k, v in values.items())
    return ordered(rows)


def ordered(rows):
    return sorted(rows, key=lambda r: json.dumps(r, sort_keys=True, ensure_ascii=False))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    parser.add_argument('--source', type=Path)
    parser.add_argument('--engine', type=Path)
    parser.add_argument('--baseline', type=Path)
    parser.add_argument('--rule', type=Path, default=ROOT/'test/fixtures/rr/survey-source.rr')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    evidence = dict(format='rerail.survey-source-inventory/v1', sourcePin=PIN,
                    fullMigrationAcceptance=False, cases=[])
    if args.command == 'source':
        if not args.source or git(args.source, 'rev-parse', 'HEAD') != PIN:
            parser.error('The pinned source checkout is required')
        evidence['sourceSha256'] = sha((args.source/'scripts/lint_lib/symbol_graph.py').read_bytes())
        for label, files in CASES:
            evidence['cases'].append(dict(id=label, files=[dict(path=p, source=t) for p,t in files.items()],
                                          expected=source_rows(args.source, files)))
    else:
        from replay import execute_native
        baseline = json.loads(args.baseline.read_text())
        if baseline['sourcePin'] != PIN:
            parser.error('The source recording pin differs')
        evidence['baselineSha256'] = sha(args.baseline.read_bytes())
        for case in baseline['cases']:
            native = execute_native(args.engine, args.rule, case['files'], {})
            rows = ordered([dict(r['parameters']) for r in native['diagnostics']])
            evidence['cases'].append(dict(id=case['id'], expected=case['expected'], actual=rows,
                                          passed=rows == case['expected'], programInputs=native['programInputs']))
    evidence['passed'] = all(c.get('passed', True) for c in evidence['cases'])
    write_json(args.output, evidence)
    print(json.dumps(dict(cases=len(evidence['cases']), passed=evidence['passed'],
                          failures=[c['id'] for c in evidence['cases'] if not c.get('passed', True)])))
    return 0 if evidence['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
