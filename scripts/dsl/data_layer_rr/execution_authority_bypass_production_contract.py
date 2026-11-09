"""Compare complete authority production plans, facts, findings, and CLI streams."""
import argparse
import contextlib
from dataclasses import asdict
import importlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

from execution_authority_bypass_port import configuration
from migration import ROOT,PIN,compare,git,read_json,sha,write_json
from replay import execute_workspace_context,write_fixture

RULE=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/execution-authority-bypass.rr'
BAD='_explicit_authority_var; explicit_execution_authority()\n'
ROOTS=['impl','data_layer','conform','tests','l2050_transaction','l1_short','l١_unicode']
CASES=[dict(id='empty',files=[]),dict(id='clean',files=[dict(path='impl/a.py',source='pass\n')]),
    *[dict(id='root-'+str(i),files=[dict(path=p+'/a.py',source=BAD)]) for i,p in enumerate(ROOTS)],
    dict(id='excluded-roots',files=[dict(path=p+'/a.py',source=BAD) for p in ['other','api','_internal','library','lX_bad']]),
    dict(id='nested-membership',files=[dict(path='impl/'+p+'/a.py',source=BAD) for p in ['normal','tests','test','conform','.hidden','tmp','.git','.venv','venv','__pycache__','.pytest_cache','node_modules','browser_build']]),
    dict(id='ordered-files-and-duplicates',files=[dict(path='impl/'+p+'.py',source=BAD+'\n_legacy_boundary_var\n') for p in ['z','a','a/deep']]),
    dict(id='syntax-error',files=[dict(path='impl/a.py',source='def :\n')]),
    dict(id='mixed-syntax',files=[dict(path='impl/a.py',source='def :\n'),dict(path='impl/b.py',source=BAD)]),
    dict(id='file-and-directory-links',files=[dict(path='impl/a.py',source=BAD),dict(path='impl/alias.py',symlink='a.py'),dict(path='impl/sub/a.py',source=BAD),dict(path='impl/link',symlink='sub')]),
    dict(id='signed-alias-scope',signed=[['impl.a','helper']],files=[dict(path='impl/a.py',source='def helper(): explicit_execution_authority()\n'),dict(path='impl/alias.py',symlink='a.py')]),
    dict(id='registry-errors',signed=[['z','f'],['z','f'],['b','a']],curated=[['a','f'],['b','a']],files=[dict(path='impl/a.py',source=BAD)]),
    dict(id='registry-empty',signed=[],curated=[],files=[]),
    dict(id='boundary-exception',files=[dict(path='l2050_transaction/execution_boundary.py',source=BAD)]),
    dict(id='extension-and-hidden-files',files=[dict(path='impl/'+p,source=BAD) for p in ['a.py','.hidden.py','a.pyi','a.PY','notes.txt']]),
]


def source_case(module,case):
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore
    with tempfile.TemporaryDirectory(prefix='authority-production-source-') as temporary:
        root=Path(temporary);write_fixture(root,case['files']);changes=dict(REPO_ROOT=root)
        if 'signed' in case:changes['EXPLICIT_EXECUTION_AUTHORITY_CALLSITES']=tuple(SimpleNamespace(module=m,function=f) for m,f in case['signed'])
        if 'curated' in case:changes['CURATED_EXPLICIT_AUTHORITY_CALLSITES']=frozenset(tuple(x) for x in case['curated'])
        with patch.multiple(module,**changes),patch.object(module.scan_repo,'__defaults__',(root,)):
            config=configuration(module);plugin=module.execution_authority_bypass_plugin(root)
            engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/facts'),extractors=module.execution_authority_bypass_fact_extractors(root),enable_fact_cache=False)
            plan=engine.plan([plugin]);view=engine.materialize((plugin,),max_workers=1)
            findings=[dict(asdict(v),path=str(v.path.relative_to(root))) for v in plugin.reporter(view)]
            facts=[f for path,bundle in sorted(view.files.items()) for f in bundle.facts.get(module.EXECUTION_AUTHORITY_BYPASS_EXTRACTOR.extractor_id,[])]
            scan=[asdict(v) for v in module.scan_repo(root)]
            cli=[]
            for argv in [[],['--check']]:
                out,err=io.StringIO(),io.StringIO()
                with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):status=module.cli_main(argv)
                cli.append(dict(argv=argv,status=status,stdout=out.getvalue(),stderr=err.getvalue()))
            return dict(config=config,expected=dict(selectedFiles=[str(p.relative_to(root)) for p in plan.files],facts=facts,findings=findings,scanRepo=scan,cli=cli,
                plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description)))


def native_case(engine,case,config):
    reports=[]
    def execute(entry=None,extra=None):
        def options(root):
            result=dict(config,env=['auth.root='+str(root)],**(extra or {}))
            if entry:result['entryPoint']=entry
            return result
        result=execute_workspace_context(engine,RULE,case['files'],options);reports.append(result);return result
    production=execute();facts=execute('_production_facts');scan=execute('scan_repo')
    selected=[r['parameters']['path'] for r in sorted(facts['diagnostics'],key=lambda r:r['parameters'].get('path_order','')) if r['code']=='AUTH_FILE']
    facts_rows=[r for r in facts['diagnostics'] if r['code']=='AUTH_BATCH_ROW']
    scan_rows=[r for r in scan['diagnostics'] if r['code']=='AUTH_BATCH_ROW']
    def values(rows):
        return [{k:r['parameters'][k] for k in ['path','line','code','message']} for r in sorted(rows,key=lambda r:tuple(r['parameters'][k] for k in ['phase','path_order','ordinal']))]
    if len(facts_rows)+len(selected)!=len(facts['diagnostics']) or len(scan_rows)!=len(scan['diagnostics']):raise ValueError('Unexpected producer rows')
    findings=[dict(linter_name='execution_authority_bypass',code=r['code'],path=r['path'],line=r['line'],message=r['message'],severity='error') for r in sorted(production['diagnostics'],key=lambda r:tuple(r['parameters'][k] for k in ['phase','path_order','ordinal']))]
    cli=[]
    for check in [False,True]:
        output=execute('_cli_report',dict(checkMode='1' if check else '0'))
        rows=output['diagnostics'];exit_rows=[r['parameters']['status'] for r in rows if r['code']=='AUTH_EXIT']
        if len(exit_rows)!=1:raise ValueError('One native CLI exit result required')
        cli.append(dict(argv=['--check'] if check else [],status=exit_rows[0],stdout=''.join(r['parameters']['text'] for r in sorted(rows,key=lambda r:tuple(r['parameters'].get(k,0) for k in ['phase','path_order','ordinal'])) if r['code']=='AUTH_OUTPUT'),stderr=''))
    metadata=production['metadata'];policies=metadata['policies'];settings=metadata['settings']
    if len(policies)!=1 or [v for k,v in settings if k=='mode']!=['block']:raise ValueError('Production policy metadata mismatch')
    import json
    descriptions=[json.loads(v) for k,v in settings if k=='description']
    if len(descriptions)!=1:raise ValueError('One production description required')
    return dict(selectedFiles=selected,facts=values(facts_rows),findings=findings,scanRepo=values(scan_rows),cli=cli,
        plugin=dict(name=policies[0]['id'].removeprefix('data-layer-').replace('-','_'),enforcement='hard-fail',description=descriptions[0])),reports


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path)
    p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args();assert a.out and 0<=a.part<a.parts
    paths=[Path(__file__),Path(__file__).with_name('execution_authority_bypass_port.py'),RULE]
    report=dict(format='rerail.authority-production/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,cases=[],programs={str(f.relative_to(ROOT)):dict(sha256=sha(f.read_bytes()),text=f.read_text()) for f in paths})
    if a.command=='source':
        a.source=a.source.absolute();assert git(a.source,'rev-parse','HEAD')==PIN
        n='scripts/lint_execution_authority_bypass.py';compare((a.source/n).read_bytes(),subprocess.check_output(['git','-C',str(a.source),'show',PIN+':'+n]))
        sys.path[:0]=[str(a.source),str(a.source/'scripts')];module=importlib.import_module('scripts.lint_execution_authority_bypass')
        for case in CASES:report['cases'].append(dict(input=case,**source_case(module,case)));write_json(a.out,report)
    else:
        if any(n.startswith(('scripts.lint_','lint_execution_authority_bypass')) for n in sys.modules):raise ValueError('Native acceptance cannot import source rules')
        baseline=read_json(a.baseline);assert baseline['passed'] and baseline['sourcePin']==PIN;compare(CASES,[c['input'] for c in baseline['cases']]);report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()))
        for case in baseline['cases'][a.part::a.parts]:
            actual,reports=native_case(a.engine.absolute(),case['input'],case['config']);report['cases'].append(dict(case,actual=actual,native=reports,passed=actual==case['expected']));write_json(a.out,report)
    for n,v in report['programs'].items():compare(v['sha256'],sha((ROOT/n).read_bytes()))
    report['passed']=all(c.get('passed',True) for c in report['cases']);write_json(a.out,report);print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
