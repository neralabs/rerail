"""Compare imperative findings and plugin results through one persistent workspace and store."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

import imperative_source_contract as contract
from migration import ROOT,PIN,MANIFEST,read_json,sha,write_json,verify_source_cohort,verify_engine_artifact
from replay import write_fixture,program_inputs

STEPS=['diamond-reversed','direct','empty','direct','legacy-insertion-phases',
       'fanout-seventy-seven','plugin-absent','direct','private-eager','duplicate-definitions','direct','direct']


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','output']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    baseline=read_json(args.baseline)
    if baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:parser.error('Passing pinned source recording required')
    if [{k:c[k] for k in contract.CASES[0]} for c in baseline['cases']]!=contract.CASES:parser.error('Complete unchanged corpus required')
    if any(n.startswith(('scripts.lint_','lint_imperative_private_functions')) for n in sys.modules):raise ValueError('Native update acceptance cannot import detectors')
    source=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,source);verify_engine_artifact(ROOT,source,args.engine)
    result=dict(format='rerail.imperative-updates/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),cases=[],complete=False,passed=False)
    write_json(args.output,result);by_id={c['id']:c for c in baseline['cases']}
    with tempfile.TemporaryDirectory(prefix='rerail-imperative-updates-') as directory:
        root=Path(directory);workspace=root/'corpus';workspace.mkdir();previous=set();runs=[]
        def execute(engine,rule,files,options):
            option_file=root/'options.json';write_json(option_file,options)
            run=subprocess.run([str(engine.absolute()),'run-rr',str(rule),str(workspace),str(root/'out'),str(option_file)],
                               capture_output=True,text=True,timeout=100,env={**os.environ,'RERAIL_TAPE_CACHE':str(root/'store.sqlite')})
            if run.returncode:raise RuntimeError(run.stderr)
            native=json.loads(run.stdout);native['programInputs']=program_inputs(native['programInputs'])
            runs.append(dict(options=options,phases=native['phases'],programInputs=native['programInputs']))
            return native
        with patch.object(contract,'execute_workspace',execute):
            for index,label in enumerate(STEPS):
                case=by_id[label];current={f['path'] for f in case['files']}
                for path in previous-current:(workspace/path).unlink()
                write_fixture(workspace,case['files']);previous=current;runs.clear()
                actual,inputs=contract.native_case(args.engine,case)
                expected={k:case['expected'][k] for k in ['findings','facts','plugin','violations']}
                passed=actual==expected
                result['cases'].append(dict(step=index,id=label,actual=actual,expected=expected,passed=passed,runs=list(runs)));write_json(args.output,result)
                print(json.dumps(dict(step=index,id=label,passed=passed)),flush=True)
    result.update(complete=True,passed=all(c['passed'] for c in result['cases']));write_json(args.output,result)
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
