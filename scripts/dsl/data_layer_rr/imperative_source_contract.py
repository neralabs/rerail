"""Compare complete imperative findings from physical files and the native graph producer."""
import argparse
from dataclasses import asdict
import importlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT,PIN,git,sha,write_json
from replay import write_fixture,execute_workspace


def case(name,files):return dict(id=name,files=[dict(path=p,source=t) for p,t in files.items()])


CASES=[
    case('empty',{}),
    case('direct',{'impl/a.py':'def public(c): c.execute()\n'}),
    case('private-eager',{'impl/connection.py':'def _connect(): pass\n'}),
    case('random-private',{'impl/a.py':'def _helper(): pass\n'}),
    case('lazy-name',{'impl/connection.py':'def _connect_lazy(): pass\n'}),
    case('lazy-return',{'impl/connection.py':'def _connect() -> LazyResult: pass\n'}),
    case('lazy-parameter',{'impl/connection.py':'def _connect(value: ServerPlan): pass\n'}),
    case('lazy-annotation-text',{'impl/connection.py':'def _connect(value: "NotLazyWhatever"): pass\n'}),
    case('abstract',{'impl/connection.py':'@abstractmethod\ndef _connect(): pass\n'}),
    case('property',{'impl/connection.py':'class Public:\n @property\n def _connect(self): pass\n'}),
    case('legacy',{'impl/connection.py':'@legacy_module_sync\ndef _connect(): pass\n'}),
    case('private-class',{'impl/connection.py':'class _Private:\n def run(self): pass\n def __init__(self): pass\n'}),
    case('architectural-annotation',{'impl/a.py':'def _helper(value: WriteTransaction): pass\n'}),
    case('quoted-annotation',{'impl/a.py':'def _helper(value: "WriteTransaction"): pass\n'}),
    case('positional-only-annotation',{'impl/a.py':'def _helper(value: WriteTransaction, /): pass\n'}),
    case('nested-functions',{'impl/a.py':'def outer(c):\n def inner(): c.execute()\n return inner\n'}),
    case('same-file-callers',{'impl/a.py':'def leaf(c): c.execute()\ndef middle(c): leaf(c)\ndef public(c): middle(c)\n'}),
    case('cross-file-callers',{'impl/a.py':'from impl.b import leaf\ndef public(c): leaf(c)\n','impl/b.py':'def leaf(c): c.execute()\n'}),
    case('reason-priority',{'impl/connection.py':'def _eager(c): _direct(c)\ndef _direct(c): c.execute()\ndef public(c): _eager(c)\n'}),
    case('cycle',{'impl/a.py':'def leaf(c): c.execute(); other(c)\ndef other(c): leaf(c)\n'}),
    case('diamond-order',{'impl/a.py':'def leaf(c): c.execute()\ndef zebra(c): leaf(c)\ndef alpha(c): leaf(c)\ndef public(c): zebra(c); alpha(c)\n'}),
    case('multi-seed-order',{'impl/a.py':'def first(c): c.fetchall()\ndef second(c): c.fetchone()\ndef public(c): second(c); first(c)\n'}),
    case('multiple-hits',{'impl/a.py':'def public(c): c.fetchone(); c.execute_grammar_extra(); c.resolve()\n'}),
    case('syntax-error',{'impl/a.py':'def bad(\n'}),
    case('source-scope',{'other/connection.py':'def _ignored(c): c.execute()\n','api/a.py':'def public(c): c.execute()\n'}),
    case('unicode-layer',{'l١٢٣٤_unicode/connection.py':'def _connect(): pass\n'}),
    case('layer-newline',{'l1234_\nignored/connection.py':'def _connect(): pass\n'}),
    case('shadowing',{'impl/a.py':'def target(c): c.execute()\ndef public(target): target()\n'}),
    case('methods',{'impl/a.py':'class Public:\n def leaf(self,c): c.execute()\n def run(self,c): self.leaf(c)\n'}),
    case('duplicate-definitions',{'impl/a.py':'def leaf(c): c.execute()\ndef public(c): leaf(c)\ndef leaf(c): pass\n'}),
    case('diamond-reversed',{'impl/a.py':'def leaf(c): c.execute()\ndef alpha(c): leaf(c)\ndef zebra(c): leaf(c)\ndef public(c): zebra(c); alpha(c)\n'}),
    case('plugin-present',{'scripts/lint_imperative_private_functions.py':'# plugin trigger\n','impl/a.py':'def leaf(c): c.execute()\ndef public(c): leaf(c)\n'}),
    case('fanout-five',{'impl/a.py':'def leaf(c): c.execute()\n'+''.join(f'def caller_{i}(c): leaf(c)\n' for i in range(5))}),
    case('fanout-nineteen',{'impl/a.py':'def leaf(c): c.execute()\n'+''.join(f'def caller_{i}(c): leaf(c)\n' for i in range(19))}),
    case('fanout-seventy-seven',{'impl/a.py':'def leaf(c): c.execute()\n'+''.join(f'def caller_{i}(c): leaf(c)\n' for i in range(77))}),
    case('legacy-insertion-phases',{'impl/a.py':'class Public:\n @legacy_sync\n def run(self,c): pass\n def run_lazy(self,c): c.execute()\n def caller(self,c): self.run_lazy(c)\n def public(self,c): self.run(c); self.caller(c)\n'}),
    case('weighted-source-order',dict({'impl/a.py':'def leaf(c): c.execute()\ndef public(c): leaf(c)\n'},
         **{f'impl/filler_{i:02}.py':'def filler(): pass\n'+'# padding\n'*(i%5) for i in range(51)})),
]


def source_case(source,item):
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_imperative_private_functions')
    with tempfile.TemporaryDirectory(prefix='rerail-offline-imperative-') as directory:
        root=Path(directory);write_fixture(root,item['files'])
        graph=module.build_symbol_graph(root=root,directories=module._default_dirs(root))
        findings=module.analyze_imperative_private_functions(graph)
        value=[dict(sid=sid,ordinal=i,reason=f.reason,detail=f.detail) for i,(sid,f) in enumerate(findings.items())]
        # The production extractor sorts findings before the reporter consumes them.
        facts=[module._finding_to_fact(sid,findings[sid],graph) for sid in sorted(findings,key=lambda s:(graph.symbols[s].path,graph.symbols[s].line))]
        # Use the real production planner, extractor factory, and reporter.
        from scripts.lint_lib.fact_engine import FactEngine
        from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
        plugin=module.imperative_private_functions_plugin(root)
        extractors=module.imperative_private_functions_fact_extractors(root)
        plan=FactEngine(root=root,fact_store=None,extractors=extractors,enable_fact_cache=False).plan([plugin])
        bundles={path:PerFileFactBundle(path=path,content_hash='offline',facts={spec.extractor_id:extractors[spec.extractor_id](path,path.read_text()) for spec in plan.facts_by_file[path]}) for path in plan.files}
        violations=[]
        for violation in plugin.reporter(ProjectFactView(root=root,files=bundles,required_facts=plan.required_facts)):
            row=asdict(violation);row['path']=str(violation.path.relative_to(root));violations.append(row)
        return dict(findings=value,facts=facts,callers=[[sid,list(values)] for sid,values in graph.callers.items()],
                    plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description),
                    violations=violations,plannedFiles=[str(p.relative_to(root)) for p in plan.files])


def native_case(engine,item):
    native=execute_workspace(engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/imperative-private-functions.rr',item['files'],dict(entryPoint='analyze_source'))
    unexpected=[r for r in native['diagnostics'] if r['code'] not in {'IPF_FINDING','IPF_FACT'}]
    if unexpected:raise ValueError('Unexpected native source diagnostics: '+repr(unexpected))
    rows=sorted((r['parameters'] for r in native['diagnostics'] if r['code']=='IPF_FINDING'),key=lambda r:r['ordinal'])
    facts=[]
    for row in sorted((r['parameters'] for r in native['diagnostics'] if r['code']=='IPF_FACT'),key=lambda r:r['ordinal']):
        facts.append({k:(bool(v) if k=='private' else v) for k,v in row.items() if k!='ordinal'})
    production=execute_workspace(engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/imperative-private-functions.rr',item['files'],{})
    metadata=production['metadata'];policies=metadata['policies'];settings=metadata['settings']
    if len(policies)!=1:raise ValueError('One production policy required')
    modes=[v for k,v in settings if k=='mode'];descriptions=[json.loads(v) for k,v in settings if k=='description']
    if len(modes)!=1 or len(descriptions)!=1:raise ValueError('Complete native plugin metadata required')
    plugin=dict(name=policies[0]['id'].removeprefix('data-layer-').replace('-','_'),enforcement=modes[0],description=descriptions[0])
    severities={v.split()[0]:v.split()[1] for k,v in settings if k=='severity'}
    violations=[dict(linter_name=plugin['name'],code=r['code'],path=r['path'],line=r['line'],message=r['message'],severity=severities.get(r['clause'],'error')) for r in production['diagnostics']]
    return dict(findings=rows,facts=facts,plugin=plugin,violations=violations),[native['programInputs'],production['programInputs']]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=name=='output')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args()
    if args.parts<1 or not 0<=args.part<args.parts:parser.error('Invalid partition')
    result=dict(format='rerail.imperative-source/v1',sourcePin=PIN,fullMigrationAcceptance=False,cases=[],complete=False,passed=False)
    if args.command=='source':
        if git(args.source,'rev-parse','HEAD')!=PIN:parser.error('Pinned source required')
        names=['scripts/lint_imperative_private_functions.py','scripts/lint_lib/symbol_graph.py','scripts/lint_lib/discovery.py']
        for name in names:
            if (args.source/name).read_bytes()!=subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name]):parser.error('Modified source')
        result['sourceFiles']={name:sha((args.source/name).read_bytes()) for name in names};write_json(args.output,result)
        for item in CASES:result['cases'].append(dict(item,expected=source_case(args.source,item)));write_json(args.output,result)
        if not sum(bool(c['expected']['findings']) for c in result['cases']):raise ValueError('Positive source findings required')
    else:
        if any(n.startswith(('scripts.lint_','lint_imperative_private_functions')) for n in sys.modules):raise ValueError('Native source acceptance cannot import detectors')
        source=json.loads(args.baseline.read_text())
        if source['format']!=result['format'] or source['sourcePin']!=PIN or not source['complete'] or not source['passed']:parser.error('Complete passing source recording required')
        if [{k:c[k] for k in CASES[0]} for c in source['cases']]!=CASES:parser.error('Source corpus differs')
        result.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),caseSelection=dict(part=args.part,parts=args.parts,total=len(CASES)));write_json(args.output,result)
        for item in source['cases'][args.part::args.parts]:
            actual,inputs=native_case(args.engine,item);expected={k:item['expected'][k] for k in ['findings','facts','plugin','violations']}
            result['cases'].append(dict(id=item['id'],expected=expected,actual=actual,passed=actual==expected,programInputs=inputs));write_json(args.output,result)
    result.update(complete=True,passed=all(c.get('passed',True) for c in result['cases']));write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c.get('passed',True)])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
