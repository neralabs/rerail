"""Record complete authority scanner and registry values; replay without importing Python detectors."""
import argparse
import ast
from dataclasses import asdict
import importlib
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

from migration import ROOT,PIN,compare,git,read_json,sha,write_json
from replay import execute_native
from execution_authority_bypass_port import configuration

RULE=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/execution-authority-bypass.rr'
BOUNDARY='l2050_transaction/tests/test_execution_boundary.py'

def case(name,source='',path='impl/fake/authority_sample.py',**options):
    return dict(id=name,source=source,path=path,**options)

CASES=[case('empty'),case('strings-and-comments','"_explicit_authority_var"\n# explicit_execution_authority()\n'),
    case('bare-private','_explicit_authority_var\n_legacy_boundary_var\n'),
    case('private-attribute-chain','a._explicit_authority_var._legacy_boundary_var\n'),
    case('private-stores-and-delete','_explicit_authority_var = value\ndel obj._legacy_boundary_var\n'),
    case('private-parameter-is-not-name','def run(_explicit_authority_var): pass\n'),
    case('private-annotation','def run(x: _explicit_authority_var): pass\n'),
    case('private-import-is-not-name','from p import _explicit_authority_var\n'),
    case('private-def-is-not-name','def _explicit_authority_var(): pass\nclass _legacy_boundary_var: pass\n'),
    case('attribute-label-is-not-name','x._explicit_authority_var\n'),
    case('same-line-duplicates','_explicit_authority_var; _explicit_authority_var\n'),
    case('callee-before-receiver','a._explicit_authority_var.explicit_execution_authority()\n'),
    case('private-owner-exemption','_explicit_authority_var; explicit_execution_authority()\n',path='l2050_transaction/execution_boundary.py'),
    case('private-lookalike-path','_explicit_authority_var\n',path='nested/l2050_transaction/execution_boundary.py'),
    case('module-authority','explicit_execution_authority()\n'),
    case('attribute-authority','obj.explicit_execution_authority()\n'),
    case('parenthesized-call','((explicit_execution_authority))()\n'),
    case('subscript-callee-not-leaf','explicit_execution_authority[0]()\n'),
    case('signed-function','def helper(): explicit_execution_authority()\n',signed=[['impl.fake.authority_sample','helper']]),
    case('signed-other-function','def helper(): explicit_execution_authority()\n',signed=[['impl.fake.authority_sample','other']]),
    case('class-body','class C:\n explicit_execution_authority()\n',signed=[['impl.fake.authority_sample','C']]),
    case('class-method','class C:\n def f(self): explicit_execution_authority()\n',signed=[['impl.fake.authority_sample','C.f']]),
    case('nested-function','def f():\n def g(): explicit_execution_authority()\n',signed=[['impl.fake.authority_sample','f.<locals>.g']]),
    case('class-inside-function','def f():\n class C:\n  def g(self): explicit_execution_authority()\n',signed=[['impl.fake.authority_sample','f.C.<locals>.g']]),
    case('async-nested','async def f():\n async def g(): explicit_execution_authority()\n',signed=[['impl.fake.authority_sample','f.<locals>.g']]),
    case('lambda-transparent','def f(): return lambda: explicit_execution_authority()\n',signed=[['impl.fake.authority_sample','f']]),
    case('function-decorator','@deco(explicit_execution_authority())\ndef f(): pass\n',signed=[['impl.fake.authority_sample','f']]),
    case('class-decorator','@deco(explicit_execution_authority())\nclass C: pass\n',signed=[['impl.fake.authority_sample','C']]),
    case('defaults-and-return','def f(x=explicit_execution_authority()) -> explicit_execution_authority(): pass\n',signed=[['impl.fake.authority_sample','f']]),
    case('unicode-identifiers','def ｆ(): explicit_ｅxecution_authority(); _explicit_authority_ｖar\n',signed=[['impl.fake.authority_sample','f']]),
    *[case('private-exception-'+str(i),'def '+fn+'(): _explicit_authority_var\n',path=BOUNDARY) for i,fn in enumerate(['test_raw_explicit_authority_contextvar_value_does_not_authorize','test_explicit_execution_authority_uses_outer_callsite'])],
    case('exception-does-not-cover-nested','def test_explicit_execution_authority_uses_outer_callsite():\n def helper(): _explicit_authority_var; explicit_execution_authority()\n',path=BOUNDARY),
    case('monkeypatch-string','patcher.setattr("prefix.execution_boundary._caller_token.suffix", value)\n'),
    case('monkeypatch-concatenated','patcher.setattr("execution_boundary." "verify_callsite_authorization", value)\n'),
    case('monkeypatch-bytes','patcher.setattr(b"execution_boundary._caller_token", value)\n'),
    case('monkeypatch-dotted-target','patcher.setattr(pkg.execution_boundary, "_caller_token", value)\n'),
    case('monkeypatch-name-target','patcher.setattr(execution_boundary, "verify_callsite_authorization", value)\n'),
    case('monkeypatch-private-target','patcher.setattr(execution_boundary, "_explicit_authority_var", value)\n'),
    case('monkeypatch-wrong-target','patcher.setattr(other, "_caller_token", value)\n'),
    case('monkeypatch-wrong-marker','patcher.setattr(execution_boundary, "other", value)\n'),
    case('monkeypatch-empty','patcher.setattr()\n'),
    case('monkeypatch-bare-setattr','setattr("execution_boundary._caller_token", value)\n'),
    case('monkeypatch-first-string-precludes-second','patcher.setattr("other", "_caller_token", value)\n'),
    case('monkeypatch-keywords-before-positionals','patcher.setattr(value=value, *(execution_boundary,), marker="_caller_token")\n'),
    case('syntax-error','def broken(:\n'),
    case('path-suffix','explicit_execution_authority()\n',path='impl/fake/authority_sample.pyi',signed=[['impl.fake.authority_sample','<module>']]),
    case('registry-both-directions',entry='_registry_violations',signed=[['z','f'],['z','f'],['b','a']],curated=[['a','g'],['a','f'],['b','a']]),
    case('registry-empty',entry='_registry_violations',signed=[],curated=[]),
    case('registry-delimiter-and-unicode',entry='_registry_violations',signed=[['é','a:b'],['a:b','😀']],curated=[['é','a:b'],['a','b:😀'],['a','Ω']]),
]


CASES += [
    case('tree-overrides-clean-source','pass\n',treeSource='_explicit_authority_var\n'),
    case('tree-overrides-invalid-source','def broken(:\n',treeSource='explicit_execution_authority()\n'),
    case('tree-can-be-clean','_explicit_authority_var\n',treeSource='pass\n'),
    case('tree-no-text-trigger','pass\n',treeSource='obj._legacy_boundary_var\n'),
    case('tree-preserves-assigned-locations','pass\n',treeSource='explicit_execution_authority()\n',lineIncrement=40),
    case('tree-preserves-duplicate-occurrences','pass\n',treeSource='_explicit_authority_var\n',duplicate=True),
    case('tree-does-not-normalize-constructed-names','pass\n',treeSource='_explicit_authority_var\n',rename='_explicit_authority_ｖar'),
    case('tree-nested-class','pass\n',treeSource='def f():\n class C:\n  def g(self): explicit_execution_authority()\n',signed=[['impl.fake.authority_sample','f.C.<locals>.g']]),
    case('tree-monkeypatch','pass\n',treeSource='patcher.setattr(execution_boundary, "_caller_token", value)\n'),
    case('tree-monkeypatch-string','pass\n',treeSource='patcher.setattr("execution_boundary._caller_token", value)\n'),
]

CASES += [case('dot-extension-module','def helper(): explicit_execution_authority()\n',path='impl/.py',signed=[['impl..py','helper']]),
    case('invalid-module-stem','_explicit_authority_var\n',path='impl/..py')]
CASES += [case('module-path-'+str(i),path=value,entry='_module_for_path') for i,value in enumerate(['.py','..py','...a','..','a.','a..py','a..','a.b.c','impl//a.py','/impl/a.py','//impl/a.py','impl/./a.py','','.','a/..','a/...','a/.','impl/a/a.py'])]

CASES.append(case('tree-invalid-module-stem','pass\n',path='impl/..py',treeSource='_explicit_authority_var\n'))


def tree_argument(item):
    tree=ast.parse(item['treeSource'])
    if item.get('lineIncrement'):ast.increment_lineno(tree,item['lineIncrement'])
    if item.get('duplicate'):tree.body.append(tree.body[0])
    if 'rename' in item:tree.body[0].value.id=item['rename']
    return tree


def source_case(module,item):
    changes={}
    if 'signed' in item:changes['EXPLICIT_EXECUTION_AUTHORITY_CALLSITES']=tuple(SimpleNamespace(module=m,function=f) for m,f in item['signed'])
    if 'curated' in item:changes['CURATED_EXPLICIT_AUTHORITY_CALLSITES']=frozenset(tuple(r) for r in item['curated'])
    # Each fixture starts cold: source cache keys do not include monkeypatched registry values.
    with tempfile.TemporaryDirectory(prefix='authority-source-') as temporary, patch.object(module,'REPO_ROOT',Path(temporary)), (patch.multiple(module,**changes) if changes else patch.dict({},{})):
        options=configuration(module)
        try:
            if item.get('entry')=='_module_for_path':expected=module._module_for_path(Path(item['path']))
            else:
                values=module._registry_violations() if item.get('entry')=='_registry_violations' else module.scan_source(item['source'],rel_path=item['path'],**(dict(tree=tree_argument(item)) if 'treeSource' in item else {}))
                expected=[asdict(v) for v in values]
        except ValueError as error:expected=dict(error=dict(type='builtins.ValueError',args=list(error.args)))
        return dict(options=options,expected=expected)


def native_case(engine,item):
    if 'treeSource' in item:
        from execution_authority_bypass_port import argument_columns
        graph=argument_columns(tree_argument(item))
    else:graph={}
    entry=item.get('entry','scan_source')
    result=execute_native(engine,RULE,[] if entry in {'_registry_violations','_module_for_path'} else [dict(path='input.py',source=item['source'])],
        dict(item['options'],entryPoint=entry,argumentPath=item['path'],**graph))
    rows=result['diagnostics']
    if len(rows)==1 and rows[0]['code']=='AUTH_PATH_ERROR':return dict(error=dict(type='builtins.ValueError',args=[rows[0]['parameters']['detail']])),result
    if entry=='_module_for_path':
        if len(rows)!=1 or rows[0]['code']!='AUTH_MODULE':raise ValueError('One native module name required')
        return rows[0]['parameters']['value'],result
    if any(r['code']!='AUTH_ROW' for r in rows):raise ValueError('Unexpected authority diagnostic')
    return [{k:v for k,v in r['parameters'].items() if k!='ordinal'} for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])],result


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path)
    p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args()
    assert a.out and 0<=a.part<a.parts
    report=dict(format='rerail.authority-source-contract/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,cases=[])
    paths=[Path(__file__),Path(__file__).with_name('execution_authority_bypass_port.py'),RULE]
    report['programs']={str(f.relative_to(ROOT)):dict(sha256=sha(f.read_bytes()),text=f.read_text()) for f in paths}
    if a.command=='source':
        a.source=a.source.absolute();assert git(a.source,'rev-parse','HEAD')==PIN
        name='scripts/lint_execution_authority_bypass.py';assert (a.source/name).read_bytes()==subprocess.check_output(['git','-C',str(a.source),'show',PIN+':'+name])
        sys.path[:0]=[str(a.source),str(a.source/'scripts')];module=importlib.import_module('scripts.lint_execution_authority_bypass')
        for item in CASES:
            report['cases'].append(dict(item,**source_case(module,item)));write_json(a.out,report)
    else:
        if any(n.startswith(('scripts.lint_','lint_execution_authority_bypass')) for n in sys.modules):raise ValueError('Native run must not import source rules')
        baseline=read_json(a.baseline);assert baseline['passed'] and baseline['sourcePin']==PIN
        compare(CASES,[{k:v for k,v in c.items() if k not in {'options','expected'}} for c in baseline['cases']])
        report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()))
        for item in baseline['cases'][a.part::a.parts]:
            actual,native=native_case(a.engine.absolute(),item)
            report['cases'].append(dict(item,actual=actual,native=native,passed=actual==item['expected']));write_json(a.out,report)
    for path,details in report['programs'].items():compare(details['sha256'],sha((ROOT/path).read_bytes()))
    report['passed']=all(c.get('passed',True) for c in report['cases']);write_json(a.out,report)
    print(dict(cases=len(report['cases']),passed=report['passed']))
    raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
