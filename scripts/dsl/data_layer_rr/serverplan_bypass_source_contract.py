"""Compare real bypass scanning and production plugin results, including positives."""
import argparse
from dataclasses import asdict
import importlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT,PIN,git,sha,read_json,write_json
from replay import write_fixture,execute_native,execute_workspace


def case(name,text,path='impl/a.py',allowlist=(),signature=None,extra=None):
    files={path:text,**(extra or {})}
    return dict(id=name,files=[dict(path=p,source=s) for p,s in files.items()],scanPath=path,allowlist=list(allowlist),signature=signature)


CASES=[
    case('empty',''),
    *[case('method-'+name,'class Connection:\n def run(self): return self.'+name+'("sql")\n') for name in ['_execute_raw_sql','_execute_raw_sql_async','_query_raw_sql_async']],
    case('grammar','def public(c): return c.execute_grammar_sql(stmt)\n'),
    case('syntax-error','def broken(:\n'),
    case('module-call','_execute_raw_sql("sql")\n'),
    case('class-body','class Connection:\n c._execute_raw_sql("sql")\n'),
    case('async','async def public(c): await c._execute_raw_sql_async("sql")\n'),
    case('nested-function','class Outer:\n def run(self,c):\n  def nested(): c._execute_raw_sql("sql")\n  return nested()\n'),
    case('nested-class','def outer(c):\n class Inner:\n  c._execute_raw_sql("sql")\n  def run(self): c._query_raw_sql_async("sql")\n'),
    case('function-decorator','@decorator(c._execute_raw_sql("sql"))\ndef public(): c._execute_raw_sql_async("sql")\n'),
    case('class-decorator','@decorator(c._execute_raw_sql("sql"))\nclass Connection:\n def run(self): c._query_raw_sql_async("sql")\n'),
    case('default-and-annotation','def public(x: c._execute_raw_sql("sql") = c._query_raw_sql_async("sql")): c._execute_raw_sql_async("sql")\n'),
    case('dedupe-line','def public(c): c._execute_raw_sql(1); c._execute_raw_sql(2); c._query_raw_sql_async(3)\n'),
    case('nested-call-order','def public(c): c._execute_raw_sql(c._query_raw_sql_async(1))\n'),
    case('lambda','def public(c): return lambda: c._execute_raw_sql(1)\n'),
    case('parentheses','def public(c): return ((c._execute_raw_sql))(1)\n'),
    case('fetch-only','def public(c): c._fetchone(); c._fetchall_async()\n'),
    case('unicode-name','class Connｅction:\n def rｕn(self,c): c._execute_raw_sql(1)\n'),
    case('unicode-gate-negative','def public(c): c._ｅxecute_raw_sql(1)\n'),
    case('unicode-gate-positive','def public(c): c._ｅxecute_raw_sql(1); c._fetchone()\n'),
    case('allow-function','def public(c): c._execute_raw_sql(1)\n',allowlist=['impl.a::public']),
    case('allow-module','def public(c): c._execute_raw_sql(1)\n',allowlist=['impl.a::*']),
    case('allow-class','class Connection:\n def public(self,c): c._execute_raw_sql(1)\n',allowlist=['impl.a::Connection.*']),
    case('wrong-allowlist','def public(c): c._execute_raw_sql(1)\n',allowlist=['impl.a::other']),
    case('invalid-signature','def public(c): c._execute_raw_sql(1)\n',signature='NEEDS_SIGNING'),
    case('test-directory','def public(c): c._execute_raw_sql(1)\n',path='impl/tests/a.py'),
    case('cache-directory','def public(c): c._execute_raw_sql(1)\n',path='impl/.pytest_cache/a.py'),
    case('excluded-tmp','def public(c): c._execute_raw_sql(1)\n',path='impl/tmp/a.py'),
    case('api-excluded','def public(c): c._execute_raw_sql(1)\n',path='api/a.py'),
    case('unicode-layer','def public(c): c._execute_raw_sql(1)\n',path='l١٢٣٤_layer/a.py'),
    case('multiple-files','def public(c): c._execute_raw_sql(1)\n',extra={'impl/b.py':'def public(c): c._query_raw_sql_async(1)\n'}),
]


def source_case(source,item):
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_serverplan_bypass')
    with tempfile.TemporaryDirectory(prefix='rerail-bypass-source-') as directory:
        root=Path(directory);write_fixture(root,item['files'])
        def row(value):
            data=asdict(value)
            for key in ['path','file_path']:
                if key in data:
                    path=Path(data[key]);data[key]=str(path.relative_to(root)) if path.is_relative_to(root) else str(path).replace(str(source),'$SOURCE')
            return data
        with patch.object(module,'ALLOWLISTED_CALLSITES',{'TEST':item['allowlist']}),patch.object(module,'ALLOWLIST_SIGNATURE',item['signature'] or module.ALLOWLIST_SIGNATURE):
            api=[row(v) for v in module.scan_file(root/item['scanPath'])]
            plugin=module.serverplan_bypass_plugin(root)
            engine=module.FactEngine(root=root,fact_store=module.JsonFactStore(root/'.fact-store'),extractors=module.serverplan_bypass_fact_extractors(root),enable_fact_cache=False)
            view=engine.materialize([plugin],max_workers=1)
            violations=[row(v) for v in plugin.reporter(view)]
            return dict(api=api,violations=violations,plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description))


def native_case(engine,item):
    options=dict(allowlistEntries=item['allowlist'])
    if item['signature'] is not None:options['signature']=item['signature']
    document=next(f for f in item['files'] if f['path']==item['scanPath'])
    native=execute_native(engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-bypass.rr',[document],dict(options,entryPoint='scan_file',argumentPath=item['scanPath']))
    if any(r['code']!='SB_ROW' for r in native['diagnostics']):raise ValueError('Unexpected native scanner diagnostic')
    api=[{k:v for k,v in r['parameters'].items() if k!='ordinal'} for r in sorted(native['diagnostics'],key=lambda r:r['parameters']['ordinal'])]
    production=execute_workspace(engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-bypass.rr',item['files'],options)
    settings=production['metadata']['settings'];policies=production['metadata']['policies']
    modes=[v for k,v in settings if k=='mode'];descriptions=[json.loads(v) for k,v in settings if k=='description']
    if len(policies)!=1 or modes!=['block'] or len(descriptions)!=1:raise ValueError('Complete production metadata required')
    plugin=dict(name=policies[0]['id'].removeprefix('data-layer-').replace('-','_'),enforcement='hard-fail',description=descriptions[0])
    severities={v.split()[0]:v.split()[1] for k,v in settings if k=='severity'}
    rows=production['diagnostics']
    # The reporter's path/component order and per-file visit order are native data.
    rows=sorted(rows,key=lambda r:(r['path'].split('/'),r['parameters'].get('ordinal',0)))
    violations=[dict(linter_name=plugin['name'],code=r['code'],path=('$SOURCE/'+r['path'] if r['code']=='invalid_allowlist_signature' else r['path']),line=r['line'],message=r['message'],severity=severities[r['clause']]) for r in rows]
    return dict(api=api,violations=violations,plugin=plugin),[native['programInputs'],production['programInputs']]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=name=='output')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    if not 0<=args.part<args.parts:parser.error('Invalid partition')
    if args.source is not None:args.source=args.source.absolute()
    result=dict(format='rerail.bypass-source/v1',sourcePin=PIN,fullMigrationAcceptance=False,cases=[],complete=False,passed=False)
    if args.command=='source':
        if git(args.source,'rev-parse','HEAD')!=PIN:parser.error('Pinned source required')
        names=['scripts/lint_serverplan_bypass.py','scripts/lint_lib/ast_collectors.py','scripts/lint_lib/fact_engine.py']
        for n in names:
            if (args.source/n).read_bytes()!=subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+n]):parser.error('Modified source')
        result['sourceFiles']={n:sha((args.source/n).read_bytes()) for n in names};write_json(args.output,result)
        for item in CASES:result['cases'].append(dict(item,expected=source_case(args.source,item)));write_json(args.output,result)
    else:
        if any(n.startswith(('scripts.lint_','lint_serverplan_bypass')) for n in sys.modules):raise ValueError('Native acceptance cannot import detectors')
        baseline=read_json(args.baseline)
        if baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:parser.error('Complete passing source recording required')
        if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:parser.error('Unchanged complete corpus required')
        result.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),selection=dict(part=args.part,parts=args.parts,total=len(CASES)));write_json(args.output,result)
        for item in baseline['cases'][args.part::args.parts]:
            actual,inputs=native_case(args.engine,item);expected=item['expected']
            result['cases'].append(dict(id=item['id'],expected=expected,actual=actual,passed=actual==expected,programInputs=inputs));write_json(args.output,result)
    result.update(complete=True,passed=all(c.get('passed',True) for c in result['cases']));write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'],failures=[c['id'] for c in result['cases'] if not c.get('passed',True)])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
