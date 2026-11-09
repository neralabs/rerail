"""Replay production edits, removals, root changes and signatures in one native store."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

import serverplan_steps_production_contract as contract
from migration import ROOT,PIN,MANIFEST,compare,read_json,sha,write_json,verify_source_cohort,verify_engine_artifact
from replay import write_fixture,program_inputs

STEPS=['positive-root-0','clean','positive-root-1','empty','file-and-detector-order',
       'scope-membership','signature-invalid','signature-invalid-without-findings',
       'positive-root-0','empty','positive-root-0','positive-root-0']


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();args.engine=args.engine.absolute();baseline=read_json(args.baseline)
    assert baseline['sourcePin']==PIN and baseline['passed'];compare(contract.CASES,[c['input'] for c in baseline['cases']])
    if any(name.startswith('scripts.lint_') for name in sys.modules):raise ValueError('Native updates cannot import source detectors')
    source=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,source);verify_engine_artifact(ROOT,source,args.engine)
    result=dict(format='rerail.serverplan-steps-updates/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,
                engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),cases=[],
                harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in [Path(__file__),Path(contract.__file__)]})
    by_id={c['input']['id']:c for c in baseline['cases']};policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-steps.rr'
    with tempfile.TemporaryDirectory(prefix='steps-native-updates-') as temporary:
        root=Path(temporary);workspace=root/'corpus';workspace.mkdir();previous=set();runs=[];programs=None
        def execute(engine,rule,files,options):
            verify_engine_artifact(ROOT,source,engine)
            settings=options(workspace);option_file=root/'options.json';write_json(option_file,settings)
            out=root/('facts' if settings.get('entryPoint')=='_production_facts' else 'report')
            run=subprocess.run([str(engine),'run-rr',str(rule),str(workspace),str(out),str(option_file)],capture_output=True,text=True,timeout=100,
                               env=dict(os.environ,RERAIL_TAPE_CACHE=str(root/'store.sqlite')))
            if run.returncode:raise RuntimeError(run.stderr)
            native=json.loads(run.stdout);native['programInputs']=program_inputs(native['programInputs']);runs.append(native)
            return native
        with patch.object(contract,'execute_workspace_context',execute):
            for i,label in enumerate(STEPS):
                case=by_id[label];current={f['path'] for f in case['input']['files']}
                for path in previous-current:(workspace/path).unlink()
                write_fixture(workspace,case['input']['files']);previous=current;runs.clear()
                actual,native,producer=contract.native_case(args.engine,policy,case['input'],baseline['config'])
                for run in runs:
                    if programs is None:programs=run['programInputs']
                    compare(programs,run['programInputs'])
                result['cases'].append(dict(step=i,id=label,expected=case['expected'],actual=actual,runs=list(runs)))
                write_json(args.out,result);compare(case['expected'],actual);print(json.dumps(dict(step=i,id=label,passed=True)),flush=True)
    for path,digest in result['harnessFiles'].items():compare(digest,sha((ROOT/path).read_bytes()))
    result['passed']=True;write_json(args.out,result)


if __name__=='__main__':main()
