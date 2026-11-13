"""Run every unchanged plan-scope assertion and compare complete public observations."""
import argparse
import gzip
import importlib
import inspect
import json
from pathlib import Path
import select
import subprocess
import sys
import pytest
from migration import ROOT,PIN,git,sha,read_json,write_json,compare
from dto import canonical


def main():
    p=argparse.ArgumentParser();p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path,required=name in {'source','out'})
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');a=p.parse_args()
    source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    names=['scripts/lint_plan_scope_side_effects.py','scripts/tests/test_lint_plan_scope_side_effects.py']
    for name in names:compare((source/name).read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+name]))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_plan_scope_side_effects')
    from data_layer_audit.native_guard import codes
    originals=codes({'plan_scope_side_effects':module});native=[];fallbacks=[];worker=None
    if a.command=='native':
        worker=subprocess.Popen([sys.executable,str(Path(__file__).with_name('replay.py')),
            '--workspace-worker'if a.mode=='workspace'else '--worker',str(a.engine.absolute())],cwd=ROOT,
            text=True,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        class Backend:
            def replace(self,owner,name,value):setattr(owner,name,value)
            def native(self,check,entry,files,options=None,*,root=None):
                from policy_routes import policy_filename
                request=dict(rule=str(ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/policy_filename(check,entry)),options=dict(entryPoint=entry,**(options or {})))
                if root is None:request['files']=files
                else:
                    if files:raise ValueError('Physical resources are not copied into a document batch')
                    request['root']=str(root.absolute())
                worker.stdin.write(json.dumps(request)+'\n');worker.stdin.flush()
                if not select.select([worker.stdout],[],[],100)[0]:raise RuntimeError('Original assertion native request exceeded 100 seconds')
                line=worker.stdout.readline()
                if not line:raise RuntimeError('Native worker ended: '+worker.stderr.read())
                reply=json.loads(line)
                if 'error'in reply:raise RuntimeError(reply['error'])
                compare(reply['executionMode'],a.mode);result=reply['result'];native.append(dict(result,entry=entry));return result['diagnostics']
        from side_effects_scanners import install
        install(Backend(),module)
    class Plugin:
        def __init__(self):self.item=None;self.collected=[];self.cases={};self.observations=[]
        def pytest_collection_modifyitems(self,items):self.collected=[i.nodeid for i in items]
        def pytest_runtest_setup(self,item):self.item=item
        @pytest.hookimpl(hookwrapper=True)
        def pytest_runtest_call(self):
            if a.command=='native':sys.setprofile(guard)
            try:yield
            finally:sys.setprofile(None)
        def pytest_runtest_logreport(self,report):self.cases.setdefault(report.nodeid,[]).append(dict(phase=report.when,outcome=report.outcome))
    plugin=Plugin()
    for name in ['scan_repo','blocking_violations']:
        function=getattr(module,name);signature=inspect.signature(function)
        def wrap(function,name,signature):
            def observed(*args,**kwargs):
                temporary=plugin.item.funcargs['tmp_path'];values=dict(signature.bind(*args,**kwargs).arguments)
                row=dict(test=plugin.item.nodeid,entry=name,arguments=canonical(values,temporary))
                if name=='scan_repo':row['files']=[dict(path=p.relative_to(values['root']).as_posix(),source=p.read_text())for p in sorted(values['root'].rglob('*.py'))]
                plugin.observations.append(row)
                try:result=function(*args,**kwargs);row['value']=canonical(result,temporary);return result
                except BaseException as error:row['error']=dict(type=type(error).__name__,args=canonical(error.args,temporary));raise
            return observed
        setattr(module,name,wrap(function,name,signature))
    def guard(frame,event,_arg):
        if event=='call'and frame.f_code in originals and not originals[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantic function invoked: '+frame.f_code.co_qualname)
    paths=[Path(__file__),*[Path(__file__).with_name(n)for n in ['side_effects_scanners.py','policy_routes.py','migration.py','replay.py','dto.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes())for p in paths}
    try:code=int(pytest.main(['--rootdir',str(source),'-c',str(source/'pyproject.toml'),'-q',str(source/names[1])],plugins=[plugin]))
    finally:
        if worker:worker.stdin.close();worker.wait(timeout=10)
    report=dict(format='rerail.side-effects-original/v1',sourcePin=PIN,fullMigrationAcceptance=False,mode=a.command,executionMode=a.mode,
        exitCode=code,collected=plugin.collected,cases=plugin.cases,observations=plugin.observations,
        sourceFiles={name:sha((source/name).read_bytes())for name in names},harnessFiles=hashes,semanticFallbacks=fallbacks,native=native)
    if a.command=='native':
        baseline=read_json(a.baseline);compare(baseline['exitCode'],0)
        report['engineSha256']=sha(a.engine.read_bytes());report['observationsMatch']=report['observations']==baseline['observations']
        compare(report['collected'],baseline['collected']);compare(report['sourceFiles'],baseline['sourceFiles'])
        if not report['observationsMatch']or fallbacks:report['exitCode']=code=1
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    write_json(a.out,report);print(dict(cases=len(plugin.cases),exitCode=code));raise SystemExit(code)


if __name__=='__main__':main()
