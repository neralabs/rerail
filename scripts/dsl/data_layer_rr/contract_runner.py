"""Run source-recorded API contracts through either native execution interface."""
import argparse
import importlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from migration import ROOT,PIN,compare,git,read_json,write_json,sha
from validate_supplied_facts_checkpoint import equal


def main(check,format,build_cases,exercise,install,dependencies,*,guard_modules=()):
    p=argparse.ArgumentParser(description='Record or verify complete portable rule contracts.')
    p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path,required=name in {'source','out'})
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');p.add_argument('--kind',action='append')
    p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args()
    if not 0<=a.part<a.parts:p.error('Invalid shard')
    source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN);path=source/f'scripts/lint_{check}.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+path.relative_to(source).as_posix()]))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+check)
    extra={name:importlib.import_module(name) for name in guard_modules}
    extra_files={}
    for name,owner in extra.items():
        file=Path(owner.__file__);relative=file.relative_to(source).as_posix()
        compare(file.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+relative]))
        extra_files[relative]=sha(file.read_bytes())
    corpus=build_cases(module);fallbacks=[]
    if a.command=='native':
        cache=tempfile.TemporaryDirectory(prefix='rerail-contract-cache-')
        os.environ['RERAIL_TAPE_CACHE']=str(Path(cache.name)/'store.sqlite')
        a.engine=a.engine.absolute()
        from replay import execute_native,execute_workspace
        from data_layer_audit.native_guard import codes
        from policy_routes import policy_filename
        original_codes=codes({check:module,**extra});execute=execute_native if a.mode=='snapshot' else execute_workspace
        class Backend:
            def __init__(self):self.calls=[]
            def replace(self,owner,name,value):setattr(owner,name,value)
            def sign_ed25519(self,requests):
                from replay import sign_native
                result=sign_native(a.engine,requests)
                self.calls.append(dict(utility='sign-ed25519',requests=requests,replies=result))
                return result
            def native(self,check,entry,files,options=None,*,root=None):
                policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/policy_filename(check,entry)
                arguments=dict(({} if entry is None else dict(entryPoint=entry)),**(options or {}))
                if root is None:result=execute(a.engine,policy,files,arguments)
                else:
                    from replay import execute_repository
                    if files:raise ValueError('Repository transport takes physical resources, not document copies')
                    result=execute_repository(a.engine,policy,root,arguments,a.mode)
                self.calls.append(result);return result['diagnostics']
        backend=Backend();install(backend,module,detectors=True)
        baseline=read_json(a.baseline);compare(baseline['passed'],True);compare(baseline.get('additionalSourceFiles',{}),extra_files)
        compare([r['input'] for r in baseline['cases']],corpus);compare(baseline['sourceSha256'],sha(path.read_bytes()))
        def guard(frame,event,_arg):
            if event=='call' and frame.f_code in original_codes and not original_codes[frame.f_code][2]:
                fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed: '+frame.f_code.co_qualname)
    paths=[Path(__file__),*[(Path(__file__).parent/n).resolve() for n in ['migration.py','replay.py','dto.py','policy_routes.py',*dependencies]]]
    # Bind every imported local helper, including adapters reached by installation.
    for owner in tuple(sys.modules.values()):
        filename=getattr(owner,'__file__',None)
        if filename:
            file=Path(filename).resolve()
            if file.suffix=='.py' and file.is_relative_to(Path(__file__).resolve().parent.parent):paths.append(file)
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths if p.exists()}
    report=dict(format=format,sourcePin=PIN,sourceSha256=sha(path.read_bytes()),mode=a.command,executionMode=a.mode,
        passed=False,fullMigrationAcceptance=False,additionalSourceFiles=extra_files,harnessFiles=hashes,totalCases=len(corpus),part=a.part,parts=a.parts,cases=[])
    if a.command=='native':report.update(oracle=False,engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),semanticFallbacks=fallbacks)
    for i,case in enumerate(corpus):
        if i%a.parts!=a.part or (a.kind and case['kind'] not in a.kind):continue
        if a.command=='native':sys.setprofile(guard)
        try:
            with tempfile.TemporaryDirectory(prefix=check+'-contract-') as temporary:actual=exercise(module,case,Path(temporary))
        finally:sys.setprofile(None)
        row=dict(input=case)
        if a.command=='source':row['expected']=actual
        else:
            expected=baseline['cases'][i]['expected'];row.update(expected=expected,actual=actual,passed=equal(actual,expected),native=list(backend.calls));backend.calls.clear()
        report['cases'].append(row);write_json(a.out,report)
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=all(r.get('passed',True) for r in report['cases']) and not fallbacks
    write_json(a.out,report);print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)
