"""Replay direct-execution edits, path aliases, and unchanged repeats in one persistent native workspace."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
from unittest.mock import patch

import impl_direct_execution_production_contract as contract
from migration import ROOT,PIN,MANIFEST,compare,read_json,sha,write_json,verify_source_cohort,verify_engine_artifact
from replay import write_fixture,program_inputs

STEPS=['empty','root-0','clean','root-4','nested-membership','allowed-files','ordered-multiple','links','alias-allowlist-isolation','component-path-order','patterns-messages','empty','root-0','root-0','empty']


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['engine','baseline','out']:p.add_argument('--'+n,type=Path,required=True)
    a=p.parse_args();a.engine=a.engine.absolute();source=read_json(a.baseline);assert source['passed'] and source['sourcePin']==PIN
    compare(contract.CASES,[c['input'] for c in source['cases']]);cohort=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,cohort);verify_engine_artifact(ROOT,cohort,a.engine)
    report=dict(format='rerail.impl-direct-updates/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),cases=[])
    cases={c['input']['id']:c for c in source['cases']}
    with tempfile.TemporaryDirectory(prefix='impl-direct-updates-') as temporary:
        root=Path(temporary);workspace=root/'corpus';workspace.mkdir();previous={};executions=[];programs=None
        def execute(engine,rule,files,options):
            verify_engine_artifact(ROOT,cohort,engine);option_file=root/'options.json';write_json(option_file,options)
            run=subprocess.run([str(engine),'run-rr',str(rule),str(workspace),str(root/'out'),str(option_file)],capture_output=True,text=True,timeout=100,env=dict(os.environ,RERAIL_TAPE_CACHE=str(root/'store.sqlite')))
            if run.returncode:raise RuntimeError(run.stderr)
            result=json.loads(run.stdout);result['programInputs']=program_inputs(result['programInputs']);executions.append(result);return result
        with patch.object(contract,'execute_workspace',execute):
            for step,label in enumerate(STEPS):
                case=cases[label];current={f['path']:f for f in case['input']['files']};changed={p for p in set(previous)|set(current) if previous.get(p)!=current.get(p)}
                for path in sorted((p for p in previous if p in changed),key=lambda p:p.count('/'),reverse=True):(workspace/path).unlink()
                write_fixture(workspace,[f for p,f in current.items() if p in changed]);executions.clear();before={p:(workspace/p).lstat().st_mtime_ns for p in current}
                actual,_=contract.native_case(a.engine,case['input']);compare(case['expected'],actual)
                for e in executions:
                    if programs is None:programs=e['programInputs']
                    compare(programs,e['programInputs'])
                if step and label==STEPS[step-1]:assert not changed and before==prior_times
                report['cases'].append(dict(step=step,id=label,changedPaths=sorted(changed),expected=case['expected'],actual=actual,native=list(executions),passed=True));write_json(a.out,report);print(dict(step=step,id=label,passed=True),flush=True)
                previous=current;prior_times=before
    report['passed']=True;write_json(a.out,report)


if __name__=='__main__':main()
