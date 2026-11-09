"""Verify bypass API and plugin results through edits and option changes in one store."""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

import serverplan_bypass_source_contract as contract
from migration import ROOT,PIN,MANIFEST,read_json,sha,write_json,verify_source_cohort,verify_engine_artifact
from replay import write_fixture,program_inputs

STEPS=['method-_execute_raw_sql','grammar','method-_execute_raw_sql_async','syntax-error',
       'function-decorator','allow-function','wrong-allowlist','invalid-signature',
       'test-directory','unicode-gate-positive','multiple-files','method-_execute_raw_sql','method-_execute_raw_sql']


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();baseline=read_json(args.baseline)
    if baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:parser.error('Complete source recording required')
    if [{k:c[k] for k in contract.CASES[0]} for c in baseline['cases']]!=contract.CASES:parser.error('Complete unchanged corpus required')
    if any(n.startswith(('scripts.lint_','lint_serverplan_bypass')) for n in sys.modules):raise ValueError('Native updates cannot import detectors')
    source=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,source);verify_engine_artifact(ROOT,source,args.engine)
    result=dict(format='rerail.bypass-updates/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),cases=[],complete=False,passed=False)
    write_json(args.output,result);by_id={c['id']:c for c in baseline['cases']}
    with tempfile.TemporaryDirectory(prefix='rerail-bypass-updates-') as directory:
        root=Path(directory);workspace=root/'corpus';workspace.mkdir();previous=set();runs=[]
        def execute(engine,rule,files,options,*,direct):
            value=dict(options)
            if direct:value['inputFiles']=[f['path'] for f in files]
            option_file=root/'options.json';write_json(option_file,value)
            run=subprocess.run([str(engine.absolute()),'run-rr',str(rule),str(workspace),str(root/'out'),str(option_file)],
                               capture_output=True,text=True,timeout=100,env={**os.environ,'RERAIL_TAPE_CACHE':str(root/'store.sqlite')})
            if run.returncode:raise RuntimeError(run.stderr)
            native=json.loads(run.stdout);native['programInputs']=program_inputs(native['programInputs'])
            runs.append(dict(direct=direct,options=value,phases=native['phases'],programInputs=native['programInputs']))
            return native
        with ExitStack() as stack:
            stack.enter_context(patch.object(contract,'execute_native',lambda *a,**k:execute(*a,**k,direct=True)))
            stack.enter_context(patch.object(contract,'execute_workspace',lambda *a,**k:execute(*a,**k,direct=False)))
            for index,label in enumerate(STEPS):
                item=by_id[label];current={f['path'] for f in item['files']}
                for path in previous-current:(workspace/path).unlink()
                write_fixture(workspace,item['files']);previous=current;runs.clear()
                actual,inputs=contract.native_case(args.engine,item);expected=item['expected'];passed=actual==expected
                result['cases'].append(dict(step=index,id=label,actual=actual,expected=expected,passed=passed,runs=list(runs)));write_json(args.output,result)
                print(json.dumps(dict(step=index,id=label,passed=passed)),flush=True)
    result.update(complete=True,passed=all(c['passed'] for c in result['cases']));write_json(args.output,result)
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
