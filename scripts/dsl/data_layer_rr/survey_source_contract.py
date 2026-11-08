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
    ('signature-kinds', {'a.py': 'def f(pos: int, /, x: list[int], y=1, *args: str, z: A | None = None, **kwargs: object) -> tuple[int, str]: pass\nclass C:\n def m(self, cls: int, x: str, *, self: int = 0, cls: bool = True): pass\n'}),
    ('signature-empty', {'a.py': 'def f(): pass\nclass C:\n def m(self, /, *args, **kwargs): pass\n'}),
    ('signature-canonical', {'a.py': 'async def f(x: (((A| B))), y: "A") -> dict [str,list[tuple[A,B]]]: pass\n'}),
    ('module-refs', {'a.py': 'import ignored\nvalue = target\ndef omitted(x=NoDefault) -> NoReturn:\n return NoBody\nclass C(NoBase):\n field: FieldType = DefaultValue\n @decorate(DecoratorArg)\n def m(self, x: NoParam = MethodDefault, *, y=KeywordDefault) -> NoResult:\n  return NoMethodBody\n'}),
    ('conditional-module-refs', {'a.py': 'if flag:\n def f(x=Default):\n  return IncludedBody\nclass C:\n if flag:\n  def m(self, x=OtherDefault):\n   return IncludedMethodBody\n class Nested(Base):\n  def n(self): return NestedBody\n'}),
    ('module-ref-bindings', {'a.py': 'values = [x for x in source if pred(x)]\na: Alias = factory()\nclass C:\n store = target\n def m(self, /, x=PosDefault, *, y=KwDefault): pass\n'}),
    ('bases-and-fields', {'a.py': 'class C(pkg.ABC, Generic[A], metaclass=Meta):\n first: list[A]\n second: "B" = value\n first: C | None = None\n other.member: int = 0\n if flag:\n  hidden: int\n'}),
    ('decorator-spelling', {'a.py': '@pkg.decorate(1)\n@((factory()))\nclass C:\n @pkg.abstractmethod\n @wrap(value="(")\n def f(self): pass\n @property\n def g(self): pass\n'}),
    ('complex-decorators-and-bases', {'a.py': '@(lambda f: f)\n@(registry["key"])\nclass C(*bases, **keywords):\n @((make_decorator())(1))\n async def method(self): pass\nclass D(factory("ABC"), x if flag else y): pass\n'}),
    ('multiple-definition-files', {'impl/a.py': '@first\nclass A(ABC):\n x: int\n def method(self, x: int): pass\n', 'impl/b.py': '@second(1)\nclass B(pkg.Base):\n y: str\n def method(self, y: str): pass\n'}),
    ('body-call-shapes', {'a.py': 'def f():\n direct()\n pkg.method()\n self.method()\n cls.method()\n super().method()\n self.field.method()\n cls.field.method()\n factory().method()\n pkg.field.method()\n'}),
    ('body-argument-duplicates', {'a.py': 'def f():\n register(callback, self.method, pkg.method, key=other, **options)\n register(*spread, **keywords)\n register((callback), x=(self.method), y=(pkg.method))\n'}),
    ('body-local-order', {'a.py': 'def f(param, /, *args, keyword, **kwargs):\n unknown\n param\n args\n keyword\n kwargs\n unknown = source\n unknown\n del later\n later\n bound = bound\n bound()\n'}),
    ('body-argument-before-traversal', {'a.py': 'def f():\n consume((name := value), name)\n ((callee := factory))(callee)\n consume(inner(name), name)\n'}),
    ('body-attributes', {'a.py': 'def f():\n pkg.method\n self.method\n cls.method\n self.field.method\n super().method\n pkg.method = target\n del self.field\n registry = {"x": self.method}\n'}),
    ('body-builtins', {'a.py': 'def f():\n len(values)\n register(len, self, cls, super, range, missing)\n len\n frozenset\n pow\n len.method\n'}),
    ('body-nested-definitions', {'a.py': 'def outer():\n @decorate(callback)\n def inner(x: Annotation = Default) -> Return:\n  consume(x, callback)\n  saved = value\n saved\n inner\n class Nested(Base):\n  field: Type = factory()\n  def method(self, param=Default):\n   consume(param)\n'}),
    ('body-imports', {'pkg/a.py': 'def f():\n imported\n from .first import thing as imported\n imported\n imported()\n import pkg.module as imported\n imported\n from other import last as imported\n def nested():\n  import nested.module\n module\n'}),
    ('body-import-duplicates', {'a.py': 'def f():\n import first as alias, second as alias\n from pkg import a as same, b as same\n import final as same\n from ignored import *\n'}),
    ('body-execution-hits', {'a.py': 'def f(execute):\n execute()\n obj.execute_async()\n execute_grammar_custom()\n obj.grammar_fetch_extra()\n raw_execute()\n fetchall\n fetchone()\n unrelated()\n'}),
    ('body-comprehension-lambda', {'a.py': 'def f():\n values = [transform(item) for item in source if predicate(item)]\n callback = lambda arg: consume(arg)\n mapping = {(key := source): target(key), other: value}\n consume(key)\n'}),
    ('body-except-and-patterns', {'a.py': 'def f():\n global external\n try:\n  action()\n except Error as error:\n  consume(error)\n match subject:\n  case pkg.C(value) if predicate(value):\n   consume(value)\n'}),
    ('body-conditional-visit-order', {'a.py': 'def f():\n result = consume(name) if (name := condition) else other(name)\n consume({(key := source): value, other: consume(key)})\n'}),
    ('body-relative-imports', {'__init__.py': 'def f():\n from . import skip\n from ..base import X\n', 'pkg/__init__.py': 'def f():\n from . import A\n from ..base import B\n'}),
    ('body-non-simple-execution', {'a.py': 'def f():\n factory().grammar_fetch_opaque()\n registry[key].execute_grammar_opaque()\n self.worker.grammar_fetch_private()\n factory().execute()\n'}),
    ('body-generator-arguments', {'a.py': 'def f():\n consume(transform(item) for item in source)\n consume(x for x in source if pred(x))\n consume(*(factory(x) for x in source), **keywords)\n'}),
    ('body-more-binding-forms', {'a.py': 'def f():\n for first, *rest in source:\n  consume(first, rest)\n with manager() as resource:\n  consume(resource)\n x += x\n x\n callback = lambda *, only=default: consume(only)\n'}),
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
                             line=s.line, owner=s.class_name or '', async_flag=int(s.is_async), abstract_flag=int(s.is_abstract)))
            for kind, values in [('decorator',s.decorators),('class_decorator',s.class_decorators),('base',s.bases)]:
                rows.extend(dict(kind=kind,path=path,symbol_ordinal=i,ordinal=j,text=text) for j,text in enumerate(values))
            rows.extend(dict(kind='class_attribute',path=path,symbol_ordinal=i,ordinal=j,name=name,text=text)
                        for j,(name,text) in enumerate(s.class_attr_annotations))
            if s.kind != 'class':
                rows.append(dict(kind='return', path=path, symbol_ordinal=i, has_annotation=int(s.return_annotation is not None), text=s.return_annotation or ''))
                for j, (name, ann) in enumerate(zip(s.param_names, s.param_annotations)):
                    rows.append(dict(kind='parameter', path=path, symbol_ordinal=i, ordinal=j, name=name,
                                     has_annotation=int(ann is not None), text=ann or ''))
                rows.extend(dict(kind='raw_ref', path=path, symbol_ordinal=i, ordinal=j,
                                 ref_kind=ref.kind, base=ref.base or '', has_base=int(ref.base is not None), attr=ref.attr)
                            for j,ref in enumerate(s.raw_refs))
                rows.extend(dict(kind='execution_hit',path=path,symbol_ordinal=i,name=name) for name in s.execution_hits)
                rows.extend(dict(kind='local_import',path=path,symbol_ordinal=i,name=name,target=target)
                            for name,target in s.local_imports.items())
        for name, ids in info.defined_names.items():
            for i, sid in enumerate(ids):
                rows.append(dict(kind='defined', path=path, module=module, name=name, sid=sid, ordinal=i))
        for kind, values in [('imported', info.imported_symbols), ('import_module', info.imported_modules)]:
            rows.extend(dict(kind=kind, path=path, module=module, name=k, target=v) for k, v in values.items())
        rows.extend(dict(kind='module_ref', path=path, module=module, name=name) for name in info.module_level_names)
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
        required = [(label, [dict(path=p, source=t) for p,t in files.items()]) for label,files in CASES]
        recorded = [(case['id'], case['files']) for case in baseline['cases']]
        if baseline['sourcePin'] != PIN or baseline['format'] != evidence['format']:
            parser.error('The source recording identity differs')
        if recorded != required or any(not case['expected'] for case in baseline['cases']):
            parser.error('The complete source fixture recording is required')
        evidence['baselineSha256'] = sha(args.baseline.read_bytes())
        evidence['engineSha256'] = sha(args.engine.read_bytes())
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
