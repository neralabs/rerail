"""Compare persistent dispatch edits with independently recorded repository results."""
import argparse
import importlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from migration import ROOT,PIN,MANIFEST,compare,read_json,sha,write_json,verify_source_cohort,verify_engine_artifact
from replay import write_fixture,program_inputs
from dispatch_repository_contract import decode,cases

SEQUENCES=[['empty','generic-roots','empty-scope-directories','clean','generic-roots','generic-roots','empty'],
    ['empty','nonempty-inventory','empty-inventory','bad-signature','empty'],
    ['empty','alias-from-base','shared-source-scopes','alias-to-base','empty']]


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--mode',choices=['snapshot','workspace'],required=True);p.add_argument('--sequence',type=int,choices=range(len(SEQUENCES)),required=True);a=p.parse_args()
    source=a.source.absolute();sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    baseline=read_json(a.baseline);compare(baseline['sourcePin'],PIN);compare(baseline['passed'],True)
    compare([r['input'] for r in baseline['cases']],cases(module))
    compare(baseline['sourceSha256'],sha((source/'scripts/lint_serverplan_dispatch_architecture.py').read_bytes()))
    cohort=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,cohort);verify_engine_artifact(ROOT,cohort,a.engine.absolute())
    from data_layer_audit.native_guard import codes
    original=codes({'dispatch':module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in original and not original[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['dispatch_repository_contract.py','migration.py','replay.py','dto.py']]]
    report=dict(format='rerail.dispatch-updates/v1',sourcePin=PIN,admitted=False,complete=False,passed=False,mode=a.mode,sequence=a.sequence,
        engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),cases=[],semanticFallbacks=fallbacks,
        harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths})
    by_id={r['input']['id']:r for r in baseline['cases']};policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr'
    write_json(a.out,report)
    with tempfile.TemporaryDirectory(prefix='dispatch-updates-') as temporary:
        base=Path(temporary);workspace=base/'corpus';workspace.mkdir();previous={};programs=None
        for step,label in enumerate(SEQUENCES[a.sequence]):
            wanted=by_id[label];case=wanted['input'];current={f['path']:f for f in case['files']}
            changed={p for p in previous.keys()|current.keys() if previous.get(p)!=current.get(p)}
            for name in sorted(previous.keys()&changed,key=lambda n:n.count('/'),reverse=True):
                path=workspace/name
                if path.is_dir() and not path.is_symlink():shutil.rmtree(path)
                else:path.unlink()
                # Retain explicitly requested empty directories. Remove other
                # empty parents so each step matches its recorded disk fixture.
                parent=path.parent
                while parent!=workspace:
                    relative=parent.relative_to(workspace).as_posix()
                    if current.get(relative,{}).get('directory'):break
                    try:parent.rmdir()
                    except OSError:break
                    parent=parent.parent
            write_fixture(workspace,[f for p,f in current.items() if p in changed])
            mtimes={p:(workspace/p).lstat().st_mtime_ns for p in current}
            if step and label==SEQUENCES[a.sequence][step-1]:compare(mtimes,previous_mtimes);compare(changed,set())
            options=dict(entryPoint='_production_contract')
            if 'signature' in case:options['dispatchSignature']=case['signature']
            if a.mode=='snapshot':options['inputFiles']=[p.relative_to(workspace).as_posix() for p in workspace.rglob('*') if p.is_file()]
            write_json(base/'options.json',options);verify_engine_artifact(ROOT,cohort,a.engine.absolute())
            run=subprocess.run([str(a.engine.absolute()),'run-rr',str(policy),str(workspace),str(base/'out'),str(base/'options.json')],capture_output=True,text=True,timeout=100,env=dict(os.environ,RERAIL_TAPE_CACHE=str(base/'cache.sqlite')))
            if run.returncode:raise RuntimeError(run.stderr)
            native=json.loads(run.stdout);native['programInputs']=program_inputs(native['programInputs'])
            if programs is None:programs=native['programInputs']
            compare(programs,native['programInputs']);sys.setprofile(guard)
            try:actual=decode(module,native)
            finally:sys.setprofile(None)
            row=dict(step=step,id=label,changedPaths=sorted(changed),expected=wanted['expected'],actual=actual,native=native,passed=actual==wanted['expected'])
            report['cases'].append(row);write_json(a.out,report);print(dict(step=step,id=label,passed=row['passed']),flush=True)
            previous=current;previous_mtimes=mtimes
    report.update(complete=True,passed=all(r['passed'] for r in report['cases']) and not fallbacks);write_json(a.out,report)
    raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
