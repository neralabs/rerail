"""Compare complete native dead-code categories from source files, without a Python native fallback."""
import argparse
import ast
import importlib
import json
from pathlib import Path
import sys
import tempfile

from migration import PIN, ROOT, git, sha, write_json
from survey_source_contract import CASES as PRODUCER_CASES


CASES = [dict(id=name,files=[dict(path=p,source=t) for p,t in files.items()],extraRoots=[],excludedPaths=[])
         for name,files in PRODUCER_CASES]
for name,files,roots in [
    ('analysis-rooted-calls', {'entry.py':'def helper(): pass\ndef dead(): pass\ndef run(): helper()\n'}, ['entry::run']),
    ('analysis-rooted-instance-hint', {'entry.py':'class Worker:\n def method(self): pass\nclass Unused: pass\nclass Owner:\n def __init__(self, worker: Worker): pass\n def run(self): self.worker.method()\n'}, ['entry::Owner.run']),
    ('analysis-global-annotation-does-not-restart', {'entry.py':'def run(value: Remote): pass\n', 'remote.py':'class Remote:\n def method(self): pass\n'}, ['entry::run']),
    ('analysis-quoted-annotation', {'entry.py':'class Remote: pass\ndef run(value: "Remote"): pass\n'}, ['entry::run']),
    ('analysis-posonly-annotation', {'entry.py':'class Remote: pass\ndef run(value: Remote, /): pass\n'}, ['entry::run']),
    ('analysis-local-annotation', {'entry.py':'class Remote:\n def method(self): pass\ndef run(value: Remote | None): pass\n'}, ['entry::run']),
    ('analysis-attribute-annotation', {'entry.py':'import remote\nfrom remote import Remote\ndef run(value: remote.Remote): pass\n', 'remote.py':'class Remote:\n def method(self): pass\n'}, ['entry::run']),
    ('analysis-protocol-dispatch', {'entry.py':'class SomeProtocol:\n def method(self): pass\nclass Implementation:\n def method(self): pass\nclass Other:\n def different(self): pass\n'}, ['entry::SomeProtocol.method']),
    ('analysis-overrides-and-cobases', {'entry.py':'class Base:\n def method(self): pass\nclass Mixin:\n def method(self): pass\nclass Child(Base, Mixin):\n def method(self): pass\nclass Separate(Mixin):\n def method(self): pass\n'}, ['entry::Base.method']),
    ('analysis-legacy-reachability', {'entry.py':'class Owner:\n @legacy_sync\n def run(self): pass\n def run_lazy(self): helper()\ndef helper(): pass\ndef dead(): pass\n'}, ['entry::Owner.run']),
    ('analysis-module-import-seed', {'entry.py':'import remote\n', 'remote.py':'def used(): pass\ndef dead(): pass\nregistered = used\n'}, []),
    ('analysis-overlapping-intent-grammar', {'entry.py':'class SqlIntent(SqlGrammarBase): pass\nclass Other: pass\n'}, []),
    ('analysis-import-reference-confidence', {'entry.py':'from remote import helper\n', 'remote.py':'def helper(): pass\ndef other(): pass\n'}, []),
    ('analysis-abc-high-confidence', {'entry.py':'class Base(ABC):\n @abstractmethod\n def method(self): pass\nclass Child(Base):\n def method(self): pass\n def other(self): pass\n'}, ['entry::Base']),
]:
    CASES.append(dict(id=name,files=[dict(path=p,source=t) for p,t in files.items()],extraRoots=roots,excludedPaths=[]))
CASES.append(dict(id='analysis-excluded-path', files=[dict(path='impl/a.py',source='def dead(): pass\n'),dict(path='impl/b.py',source='def kept(): pass\n')],extraRoots=[],excludedPaths=['impl/a.py']))

MIXINS='class Base:\n def run(self): self._hook()\nclass FirstMixin:\n def _hook(self): pass\nclass SecondMixin:\n def _hook(self): pass\nclass One(Base, FirstMixin): pass\nclass Two(Base, SecondMixin): pass\n'
for name,files,roots in [
    ('analysis-mixin-alternatives',{'a.py':MIXINS},['a::Base.run']),
    ('analysis-mixin-unrooted',{'a.py':MIXINS},[]),
    ('analysis-mixin-unrelated',{'a.py':MIXINS+'class Unrelated:\n def _hook(self): pass\n'},['a::Base.run']),
    ('analysis-mixin-callback-closures',{'a.py':MIXINS.replace('def _hook(self): pass','def _hook(self): first_helper()',1).replace('def _hook(self): pass','def _hook(self): second_helper()',1)+'def first_helper(): pass\ndef second_helper(): pass\ndef orphan_helper(): pass\n'},['a::Base.run']),
    ('analysis-mixin-inherited-targets',{'a.py':'class FirstParent:\n def _hook(self): pass\nclass SecondParent:\n def _hook(self): pass\nclass FirstMixin(FirstParent): pass\nclass SecondMixin(SecondParent): pass\nclass Base:\n def run(self): self._hook()\nclass One(Base, FirstMixin): pass\nclass Two(Base, SecondMixin): pass\n'},['a::Base.run']),
    ('analysis-mixin-two-attributes',{'a.py':MIXINS.replace('def run(self): self._hook()','def run(self):\n  self._hook()\n  self._hook()\n  self._other').replace('def _hook(self): pass','def _hook(self): pass\n def _other(self): pass')},['a::Base.run']),
    ('analysis-mixin-imported-targets',{'base.py':'class Base:\n def run(self): self._hook()\n','first.py':'class FirstMixin:\n def _hook(self): pass\n','second.py':'class SecondMixin:\n def _hook(self): pass\n','entry.py':'from base import Base\nfrom first import FirstMixin\nfrom second import SecondMixin\nclass One(Base, FirstMixin): pass\nclass Two(Base, SecondMixin): pass\n'},['base::Base.run']),
    ('analysis-mixin-cycle',{'a.py':'class Base:\n def run(self): self._hook()\nclass A(B):\n def _hook(self): pass\nclass B(A):\n def _hook(self): pass\nclass One(Base, A): pass\nclass Two(Base, B): pass\n'},['a::Base.run']),
    ('analysis-rooted-constructor',{'a.py':'class C:\n def method(self): pass\ndef run(): C()\ndef dead(): pass\n'},['a::run']),
]:
    CASES.append(dict(id=name,files=[dict(path=p,source=t) for p,t in files.items()],extraRoots=roots,excludedPaths=[]))


def source_result(source,case):
    sys.path[:0]=[str(source),str(source/'scripts')]
    model=importlib.import_module('lint_lib.symbol_graph')
    linter=importlib.import_module('lint_internal_dead_code')
    with tempfile.TemporaryDirectory(prefix='rerail-offline-source-analysis-') as temporary:
        root=Path(temporary)
        scans=[]
        for file in case['files']:
            path=root/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(file['source'],encoding='utf-8')
            scans.append(model._scan_symbol_tree(root,path,ast.parse(file['source'])))
        graph=model.SymbolGraph(root)
        graph._add_scanned_modules(scans)
        graph._resolve_edges()
        graph._add_legacy_decorator_edges()
        graph._collect_data_layer_roots()
        return linter.analyze_dead_code(graph,extra_root_ids=set(case['extraRoots']),excluded_paths=tuple(case['excludedPaths']))


def native_result(diagnostics):
    unexpected=[r for r in diagnostics if r['code'] not in {'IDC_GROUP','IDC_ITEM'}]
    if unexpected:
        raise ValueError(f'Unexpected native diagnostics: {unexpected}')
    groups=sorted((r['parameters'] for r in diagnostics if r['code']=='IDC_GROUP'),key=lambda p:p['ordinal'])
    items=sorted((r['parameters'] for r in diagnostics if r['code']=='IDC_ITEM'),key=lambda p:p['ordinal'])
    return {p['category']:[r['sid'] for r in items if r['category']==p['category']] for p in groups}


def same_result(expected,actual):
    return expected==actual and list(expected)==list(actual)


def check_positives(cases):
    rows={c['id']:c['expected'] for c in cases}
    positive={
        'analysis-rooted-calls':('unused_functions','entry::dead'),
        'analysis-global-annotation-does-not-restart':('unused_functions','remote::Remote.method'),
        'analysis-quoted-annotation':('unused_other_classes','entry::Remote'),
        'analysis-posonly-annotation':('unused_other_classes','entry::Remote'),
        'analysis-overlapping-intent-grammar':('unused_intents','entry::SqlIntent'),
        'analysis-import-reference-confidence':('unused_functions','remote::helper'),
        'analysis-legacy-reachability':('unused_functions','entry::dead'),
        'analysis-excluded-path':('unused_functions','impl.b::kept'),
        'analysis-mixin-unrelated':('unused_functions','a::Unrelated._hook'),
        'analysis-mixin-unrooted':('unused_functions','a::FirstMixin._hook'),
        'analysis-mixin-callback-closures':('unused_functions','a::orphan_helper'),
        'analysis-rooted-constructor':('unused_functions','a::dead'),
    }
    clean={
        'analysis-rooted-calls':('unused_functions','entry::helper'),
        'analysis-global-annotation-does-not-restart':('unused_other_classes','remote::Remote'),
        'analysis-local-annotation':('unused_functions','entry::Remote.method'),
        'analysis-legacy-reachability':('unused_functions','entry::helper'),
        'analysis-module-import-seed':('unused_functions','remote::used'),
        'analysis-import-reference-confidence':('high_confidence_functions','remote::helper'),
        'analysis-excluded-path':('unused_functions','impl.a::dead'),
        'analysis-mixin-alternatives':('unused_functions','a::FirstMixin._hook'),
        'analysis-mixin-inherited-targets':('unused_functions','a::SecondParent._hook'),
        'analysis-mixin-callback-closures':('unused_functions','a::second_helper'),
        'analysis-rooted-constructor':('unused_functions','a::C.method'),
    }
    for label,(category,sid) in positive.items():
        if sid not in rows[label][category]:raise ValueError(f'Missing positive: {label}/{category}/{sid}')
    for label,(category,sid) in clean.items():
        if sid in rows[label][category]:raise ValueError(f'Expected rooted/excluded symbol: {label}/{category}/{sid}')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('command',choices=['source','native'])
    p.add_argument('--source',type=Path)
    p.add_argument('--engine',type=Path)
    p.add_argument('--baseline',type=Path)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--part',type=int,default=0)
    p.add_argument('--parts',type=int,default=1)
    args=p.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:p.error('Invalid corpus partition')
    result=dict(format='rerail.survey-source-analysis/v1',sourcePin=PIN,fullMigrationAcceptance=False,cases=[])
    if args.command=='source':
        if not args.source or git(args.source,'rev-parse','HEAD')!=PIN:p.error('Require the pinned source checkout')
        result['sourceFiles']={name:sha((args.source/name).read_bytes()) for name in ['scripts/lint_lib/symbol_graph.py','scripts/lint_internal_dead_code.py']}
        for case in CASES:result['cases'].append(dict(case,expected=source_result(args.source,case)))
        check_positives(result['cases'])
    else:
        from replay import execute_native
        baseline=json.loads(args.baseline.read_text())
        if baseline['format']!=result['format'] or baseline['sourcePin']!=PIN:p.error('Wrong source recording identity')
        if [{k:c[k] for k in ['id','files','extraRoots','excludedPaths']} for c in baseline['cases']]!=CASES:p.error('Require the complete current source corpus')
        check_positives(baseline['cases'])
        result.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),caseSelection=dict(part=args.part,parts=args.parts,total=len(CASES)),complete=False,passed=False)
        write_json(args.output,result)
        for case in baseline['cases'][args.part::args.parts]:
            native=execute_native(args.engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/internal-dead-code.rr',case['files'],dict(entryPoint='analyze_dead_code',extraRoots=case['extraRoots'],excludedPaths=case['excludedPaths']))
            actual=native_result(native['diagnostics'])
            result['cases'].append(dict(id=case['id'],expected=case['expected'],actual=actual,passed=same_result(case['expected'],actual),programInputs=native['programInputs']))
            write_json(args.output,result)
    result['complete']=True
    result['passed']=all(c.get('passed',True) for c in result['cases'])
    write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c.get('passed',True)])))
    return 0 if result['passed'] else 1


if __name__=='__main__':
    raise SystemExit(main())
