"""Validate authority changes, aliases, registry edits, and unchanged repeats in one persistent native store."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

import execution_authority_bypass_production_contract as contract
from migration import ROOT,PIN,MANIFEST,compare,read_json,sha,write_json,verify_source_cohort,verify_engine_artifact
from replay import write_fixture,program_inputs

STEPS=['root-0','clean','root-3','empty','ordered-files-and-duplicates','file-and-directory-links','signed-alias-scope','registry-errors','registry-empty','default-config-positive','default-config-duplicate-registry','default-config-test-exceptions','default-config-positive','default-config-positive','empty']


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['engine','baseline','out']:p.add_argument('--'+n,type=Path,required=True)
    a=p.parse_args();a.engine=a.engine.absolute();baseline=read_json(a.baseline);assert baseline['passed'] and baseline['sourcePin']==PIN
    compare(contract.CASES,[c['input'] for c in baseline['cases']])
    if any(n.startswith(('scripts.lint_','lint_execution_authority_bypass')) for n in sys.modules):raise ValueError('Native updates cannot import detectors')
    source=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,source);verify_engine_artifact(ROOT,source,a.engine)
    report=dict(format='rerail.authority-updates/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),cases=[])
    by_id={c['input']['id']:c for c in baseline['cases']}
    with tempfile.TemporaryDirectory(prefix='authority-native-updates-') as temporary:
        root=Path(temporary);workspace=root/'corpus';workspace.mkdir();previous={};runs=[];programs=None
        def execute(engine,rule,files,options):
            verify_engine_artifact(ROOT,source,engine);settings=options(workspace);option_file=root/'options.json';write_json(option_file,settings);entry=settings.get('entryPoint','production');out=root/entry
            run=subprocess.run([str(engine),'run-rr',str(rule),str(workspace),str(out),str(option_file)],capture_output=True,text=True,timeout=100,env=dict(os.environ,RERAIL_TAPE_CACHE=str(root/'store.sqlite')))
            if run.returncode:raise RuntimeError(run.stderr)
            value=json.loads(run.stdout);value['programInputs']=program_inputs(value['programInputs']);runs.append(value);return value
        with patch.object(contract,'execute_workspace_context',execute):
            for step,label in enumerate(STEPS):
                case=by_id[label];current={f['path']:f for f in case['input']['files']};changed={path for path in set(previous)|set(current) if previous.get(path)!=current.get(path)}
                for path in sorted((p for p in previous if p in changed),key=lambda p:p.count('/'),reverse=True):(workspace/path).unlink()
                write_fixture(workspace,[f for path,f in current.items() if path in changed]);runs.clear()
                before={path:(workspace/path).lstat().st_mtime_ns for path in current}
                actual,_=contract.native_case(a.engine,case['input'],case['config'])
                for result in runs:
                    if programs is None:programs=result['programInputs']
                    compare(programs,result['programInputs'])
                compare(contract.canonical_completion(case['expected']),contract.canonical_completion(actual))
                if step and label==STEPS[step-1]:
                    assert not changed and before==prior_times,'An unchanged repeat must preserve file timestamps'
                report['cases'].append(dict(step=step,id=label,changedPaths=sorted(changed),expected=case['expected'],actual=actual,native=list(runs),passed=True));write_json(a.out,report);print(dict(step=step,id=label,passed=True),flush=True)
                previous=current;prior_times=before
    report['passed']=True;write_json(a.out,report)


if __name__=='__main__':main()
