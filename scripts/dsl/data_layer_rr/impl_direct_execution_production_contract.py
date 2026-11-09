"""Compare native direct-execution production discovery, ordered facts, metadata, and complete findings."""
import argparse
from dataclasses import asdict
import importlib
from pathlib import Path
import subprocess
import sys
import tempfile

from migration import ROOT,PIN,compare,git,read_json,sha,write_json
from replay import execute_workspace,write_fixture
from impl_direct_execution_source_contract import RULE

BAD='obj._fetchone()\nobj._execute_direct_internal()\n'
CASES=[dict(id='empty',files=[]),dict(id='clean',files=[dict(path='impl/a.py',source='pass\n')]),
 *[dict(id='root-'+str(i),files=[dict(path=p+'/a.py',source=BAD)]) for i,p in enumerate(['impl','data_layer','_internal','l1234_sample','l١٢٣٤_sample','api','l123_bad','l1234_'])],
 dict(id='nested-membership',files=[dict(path='impl/'+p+'/a.py',source=BAD) for p in ['normal','tests','test','conform','venv','.venv','tmp','__pycache__','.git','.hidden','node_modules','.pytest_cache']]),
 dict(id='allowed-files',files=[dict(path=p,source=BAD) for p in ['impl/common/l2200_sql_dialect/sql_connection.py','impl/sqlite/connection.py','impl/sqlite/notallowed.py','impl/sqlite/fts/a.py','impl/sqlite/tests/test_a.py','impl/sqlite/test_a.py']]),
 dict(id='ordered-multiple',files=[dict(path='impl/'+p+'.py',source='obj.fetchall ()\nobj._fetchone()\n') for p in ['z','a','m']]),
 dict(id='component-path-order',files=[dict(path=p,source=BAD) for p in ['impl/a.py','impl/a/deep.py']]),
 dict(id='syntax-error',files=[dict(path='impl/a.py',source='def :\n'+BAD)]),
 dict(id='hidden-and-extension',files=[dict(path='impl/'+p,source=BAD) for p in ['.py','.hidden.py','normal.py','notes.txt','a.PY','a.pyi']]),
 dict(id='links',files=[dict(path='impl/a.py',source=BAD),dict(path='impl/alias.py',symlink='a.py'),dict(path='impl/sub/a.py',source=BAD),dict(path='impl/link',symlink='sub')]),
 dict(id='alias-allowlist-isolation',files=[dict(path='impl/sqlite/a.py',source=BAD),dict(path='impl/sqlite/connection.py',symlink='a.py'),dict(path='impl/sqlite/fts/a.py',symlink='../a.py')]),
 dict(id='patterns-messages',files=[dict(path='impl/a.py',source='obj.fetchall(); obj._fetchone()\nobj.fetchall ()\n'),dict(path='impl/b.py',source='obj._fetchone('+repr('x'*100)+')\n')])]

CASES.append(dict(id='logical-token-coordinates',files=[dict(path='impl/a.py',source='"prefix\u2028obj._fetchone()"\n'),dict(path='impl/b.py',source='"prefix\u2028obj._fetchone()"\néééééé = "x"\n')]))


def source_case(module,case):
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore
    with tempfile.TemporaryDirectory(prefix='impl-direct-production-source-') as temporary:
        root=Path(temporary);write_fixture(root,case['files']);plugin=module.impl_direct_execution_plugin(root)
        engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/facts'),extractors=module.impl_direct_execution_fact_extractors(root),enable_fact_cache=False)
        plan=engine.plan([plugin]);view=engine.materialize((plugin,),max_workers=1)
        facts=[r for _,bundle in sorted(view.files.items()) for r in bundle.facts.get(module.IMPL_DIRECT_EXECUTION_EXTRACTOR.extractor_id,[]) or []]
        return dict(selectedFiles=[str(p.relative_to(root)) for p in plan.files],facts=facts,findings=[dict(asdict(v),path=str(v.path.relative_to(root))) for v in plugin.reporter(view)],plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description))


def native_case(engine,case):
    results=[execute_workspace(engine,RULE,case['files'],options) for options in [{},dict(entryPoint='_production_facts')]]
    production,facts=results;rows=facts['diagnostics']
    selected=[r['parameters']['path'] for r in sorted((r for r in rows if r['code']=='IDE_FILE'),key=lambda r:r['parameters']['path_order'])]
    if any(r['code'] not in {'IDE_FILE','IDE_ROW'} for r in rows):raise ValueError('Unexpected native fact output')
    values=[{k:r['parameters'][k] for k in ['file','line_num','line','pattern']} for r in sorted((r for r in rows if r['code']=='IDE_ROW'),key=lambda r:tuple(r['parameters'][k] for k in ['file_order','phase','ordinal']))]
    if any(r['code']!='direct_execution' for r in production['diagnostics']):raise ValueError('Unexpected production finding')
    findings=[dict(linter_name='impl_direct_execution',code=r['code'],path=r['path'],line=r['line'],message=r['message'],severity='error') for r in sorted(production['diagnostics'],key=lambda r:(r['path'],r['line'],r['message']))]
    import json
    settings=production['metadata']['settings'];assert [v for k,v in settings if k=='mode']==['warn']
    policies=production['metadata']['policies'];assert policies==[dict(id='data-layer-impl-direct-execution',revision=1)]
    description=[json.loads(v) for k,v in settings if k=='description'];assert len(description)==1
    return dict(selectedFiles=selected,facts=values,findings=findings,plugin=dict(name=policies[0]['id'].removeprefix('data-layer-').replace('-','_'),enforcement='warning',description=description[0])),results


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path)
    a=p.parse_args();report=dict(format='rerail.impl-direct-production-contract/v1',sourcePin=PIN,passed=False,fullMigrationAcceptance=False,cases=[])
    if a.command=='source':
        source=a.source.absolute();assert git(source,'rev-parse','HEAD')==PIN
        name='scripts/lint_impl_direct_execution.py';compare((source/name).read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+name]))
        sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_impl_direct_execution')
        for c in CASES:report['cases'].append(dict(input=c,expected=source_case(module,c)))
    else:
        if any(n.endswith('lint_impl_direct_execution') for n in sys.modules):raise ValueError('Native production tests cannot import source detectors')
        source=read_json(a.baseline);assert source['passed'];compare(CASES,[c['input'] for c in source['cases']]);report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),ruleSha256=sha(RULE.read_bytes()))
        for c in source['cases']:
            actual,native=native_case(a.engine.absolute(),c['input']);report['cases'].append(dict(c,actual=actual,native=native,passed=actual==c['expected']));write_json(a.out,report)
    report['passed']=all(c.get('passed',True) for c in report['cases']);write_json(a.out,report);print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
