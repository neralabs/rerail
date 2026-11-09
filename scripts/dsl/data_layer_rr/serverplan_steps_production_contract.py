"""Compare production planning, fact extraction, and reporting on physical workspaces."""
import argparse
import contextlib
from dataclasses import asdict
import importlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT,PIN,compare,git,read_json,sha,write_json
from replay import execute_workspace_context,write_fixture
from serverplan_steps_cli_contract import BAD
from serverplan_steps_port import scan_rows

DIRS=['impl/common/'+name+'/steps' for name in ['l2070_server_plan','l7300_sql_versioned_transactions','l9700_sql_graph_query']]
CASES=[dict(id='empty',files=[]),dict(id='clean',files=[dict(path=DIRS[0]+'/a.py',source='pass\n')]),
    *[dict(id='positive-root-'+str(i),files=[dict(path=directory+'/a.py',source=BAD)]) for i,directory in enumerate(DIRS)],
    dict(id='file-and-detector-order',files=[dict(path=DIRS[0]+'/'+name+'.py',source=BAD+'\nMISC_RUNTIME = 1\n') for name in ['z','a','a/deep']]),
    dict(id='scope-membership',files=[dict(path=path,source=BAD) for path in [DIRS[0]+'/a.py',DIRS[0]+'/test_a.py',DIRS[0]+'/tests/a.py',DIRS[0]+'/test/a.py',DIRS[0]+'/.hidden/a.py',DIRS[0]+'/tmp/a.py','l2070_batching/post_op.py','other/a.py']]),
    dict(id='parse-error',files=[dict(path=DIRS[0]+'/a.py',source='def :\n')]),
    dict(id='positive-plus-parse-error',files=[dict(path=DIRS[0]+'/a.py',source=BAD),dict(path=DIRS[1]+'/b.py',source='def :\n')]),
    dict(id='signature-invalid',signature='tampered',files=[dict(path=DIRS[0]+'/a.py',source=BAD)]),
    dict(id='signature-invalid-without-findings',signature='tampered',files=[]),
    dict(id='signature-unsigned',signature='NEEDS_SIGNING',files=[dict(path=DIRS[0]+'/a.py',source=BAD)]),
    dict(id='all-directory-exclusions',files=[dict(path=DIRS[0]+'/'+name+'/a.py',source=BAD) for name in ['normal','__pycache__','.pytest_cache','node_modules','.git','.venv','venv','tmp','browser_build','test','tests','conform']]),
    dict(id='file-links-and-directory-links',files=[dict(path=DIRS[0]+'/a.py',source=BAD),dict(path=DIRS[0]+'/alias.py',symlink='a.py'),dict(path=DIRS[0]+'/sub/a.py',source=BAD),dict(path=DIRS[0]+'/link',symlink='sub')]),
]


def native_case(engine,rule,case,config):
    workspace=[]
    def options(root):
        workspace.append(root);return dict(config,rootPath=str(root),signature=case.get('signature',config['signature']))
    native=execute_workspace_context(engine,rule,case['files'],options)
    producer=execute_workspace_context(engine,rule,case['files'],lambda root:dict(options(root),entryPoint='_production_facts'))
    if any(r['code'] not in {'SSP_ROW','SSP_PRODUCTION_FILE'} for r in producer['diagnostics']):raise ValueError('Unexpected producer result')
    selected=[r['parameters']['file_path'] for r in sorted((r for r in producer['diagnostics'] if r['code']=='SSP_PRODUCTION_FILE'),key=lambda r:r['parameters']['path_order'])]
    file_order={path:i for i,path in enumerate(selected)}
    values=scan_rows([r for r in producer['diagnostics'] if r['code']=='SSP_ROW']);values.sort(key=lambda v:file_order[v['file']])
    facts=[dict(code=v['code'],file=v['file'],line=v['line'],message=v['message']) for v in values]
    if any(r['clause'] not in {'StepFinding','StepSignatureFailure'} for r in native['diagnostics']):raise ValueError('Unexpected production diagnostic')
    ordered=sorted(native['diagnostics'],key=lambda r:(r['parameters'].get('path_order',r['path']),*[r['parameters'].get(k,0) for k in ['check_order','class_order','method_order','callback_order','node_order','suborder']]))
    violations=[dict(linter_name='serverplan_steps',code=r['code'],path=r['path'].removeprefix(str(workspace[0])+'/'),line=r['line'],message=r['message'],severity='error') for r in ordered]
    errors=''.join(r['parameters'].get('verification_stderr','') for r in ordered)
    actual=dict(files=len(selected),selectedFiles=selected,facts=facts,violations=violations,stdout='',stderr=errors)
    return actual,native,producer


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=name in ['source','out'])
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    args.source=args.source.absolute()
    if args.engine:args.engine=args.engine.absolute()
    assert 0<=args.part<args.parts and git(args.source,'rev-parse','HEAD')==PIN
    filename='scripts/lint_serverplan_steps.py'
    compare((args.source/filename).read_bytes(),subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+filename]))
    report=dict(format='rerail.serverplan-steps-production/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,cases=[],
        selection=dict(part=args.part,parts=args.parts,total=len(CASES)),
        harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in [Path(__file__),*[Path(__file__).with_name(n) for n in ['serverplan_steps_port.py','serverplan_steps_cli_contract.py','replay.py','migration.py','policy_routes.py']]]},
        sourceRelocation='ROOT, STEP_DIRS and __file__ are rebound to the physical fixture. Collector and reporter implementations stay unchanged.')
    if args.command=='source':
        sys.path[:0]=[str(args.source),str(args.source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_steps')
        from scripts.lint_lib.fact_engine import FactEngine
        from scripts.lint_lib.fact_store import JsonFactStore
        config=dict(signature=module.ALLOWLIST_SIGNATURE,canBatchAllowlist=sorted(module.CAN_BATCH_FALSE_ALLOWLIST),
            runtimeStateAllowlist=sorted(module.LIFECYCLE_RUNTIME_STATE_ALLOWLIST),sideEffectAllowlist=sorted(module.LEGACY_SIDE_EFFECT_TAG_ALLOWLIST),
            fateNames=list(module.POST_OP_FATE_OVERRIDES),fateValues=[v.value for v in module.POST_OP_FATE_OVERRIDES.values()])
    else:
        baseline=read_json(args.baseline);assert baseline['passed'];compare(CASES,[c['input'] for c in baseline['cases']]);config=baseline['config']
        report['engineSha256']=sha(args.engine.read_bytes())
    report['config']=config
    for index in range(args.part,len(CASES),args.parts):
        case=CASES[index]
        if args.command=='source':
            with tempfile.TemporaryDirectory(prefix='steps-production-source-') as temporary:
                root=Path(temporary);write_fixture(root,case['files']);output,errors=io.StringIO(),io.StringIO()
                with patch.object(module,'ROOT',root),patch.object(module,'STEP_DIRS',tuple(root/p for p in DIRS)),patch.object(module,'__file__',str(root/filename)),patch.object(module,'ALLOWLIST_SIGNATURE',case.get('signature',config['signature'])),contextlib.redirect_stdout(output),contextlib.redirect_stderr(errors):
                    plugin=module.serverplan_steps_plugin(root)
                    engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/facts'),extractors=module.serverplan_steps_fact_extractors(root),enable_fact_cache=False)
                    plan=engine.plan([plugin]);view=engine.materialize((plugin,),max_workers=1)
                    violations=[dict(asdict(v),path=str(v.path.relative_to(root))) for v in plugin.reporter(view)]
                    facts=[dict(f,file=str(Path(f['file']).relative_to(root))) for p,b in sorted(view.files.items()) for f in b.facts.get(module.SERVERPLAN_STEPS_EXTRACTOR.extractor_id,[])]
                expected=dict(files=len(plan.files),selectedFiles=[str(p.relative_to(root)) for p in plan.files],facts=facts,violations=violations,stdout=output.getvalue(),stderr=errors.getvalue())
            row=dict(input=case,expected=expected)
        else:
            actual,native,producer=native_case(args.engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps.rr',case,config)
            row=dict(input=case,expected=baseline['cases'][index]['expected'],actual=actual,native=native,producer=producer)
        report['cases'].append(row);write_json(args.out,report)
        if args.command=='native':compare(row['expected'],row['actual'])
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;write_json(args.out,report);print(dict(cases=len(report['cases']),passed=True))


if __name__=='__main__':main()
