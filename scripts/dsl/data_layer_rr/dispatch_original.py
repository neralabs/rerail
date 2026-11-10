"""Run unchanged dispatch assertions and record complete source/native scan observations."""
import argparse
import importlib
from pathlib import Path
import subprocess
import sys
import pytest
from migration import ROOT,PIN,git,sha,read_json,write_json,compare
from dto import canonical


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path,required=name in ['source','out'])
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1)
    a=p.parse_args();source=a.source.absolute();out=a.out.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    test='scripts/tests/test_lint_serverplan_dispatch_architecture.py';script='scripts/lint_serverplan_dispatch_architecture.py'
    for name in [test,script]:compare((source/name).read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+name]))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    from data_layer_audit.native_guard import codes
    original_codes=codes({'serverplan_dispatch_architecture':module});native=[];fallbacks=[]
    if a.command=='native':
        from replay import execute_native,execute_workspace
        from dispatch_port import install
        execute=execute_native if a.mode=='snapshot' else execute_workspace
        class Backend:
            def replace(self,owner,name,value):setattr(owner,name,value)
            def native(self,check,entry,files,options=None):
                r=execute(a.engine.absolute(),ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr',files,dict(entryPoint=entry,**(options or {})))
                native.append(r);return r['diagnostics']
        install(Backend(),module,detectors=True);baseline=read_json(a.baseline)
        compare(baseline['exitCode'],0)
    class Plugin:
        def __init__(self):self.item=None;self.cases={};self.collected=[];self.observations=[]
        def pytest_collection_modifyitems(self,items):self.collected=[i.nodeid for i in items];items[:]=items[a.part::a.parts]
        def pytest_runtest_setup(self,item):self.item=item.nodeid
        @pytest.hookimpl(hookwrapper=True)
        def pytest_runtest_call(self):
            if a.command=='native':sys.setprofile(guard)
            try:yield
            finally:sys.setprofile(None)
        def pytest_runtest_logreport(self,report):self.cases.setdefault(report.nodeid,[]).append(dict(phase=report.when,outcome=report.outcome))
    plugin=Plugin();call=module.scan_source
    def observe(path,text):
        row=dict(test=plugin.item,path=str(path),source=text);plugin.observations.append(row)
        try:result=call(path,text);row['value']=canonical(result);return result
        except BaseException as error:row['error']=dict(type=type(error).__name__,args=canonical(error.args));raise
    module.scan_source=observe
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in original_codes and not original_codes[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed: '+frame.f_code.co_qualname)
    files=[Path(__file__),*[Path(__file__).with_name(n) for n in ['dispatch_port.py','migration.py','replay.py','dto.py']]]
    hashes={f.relative_to(ROOT).as_posix():sha(f.read_bytes()) for f in files}
    code=int(pytest.main(['--rootdir',str(source),'-c',str(source/'pyproject.toml'),'-q',str(source/test)],plugins=[plugin]))
    report=dict(format='rerail.dispatch-original/v1',sourcePin=PIN,oracle=a.command=='source',mode=a.mode,exitCode=code,
        part=a.part,parts=a.parts,collected=plugin.collected,cases=plugin.cases,observations=plugin.observations,
        sourceFiles={name:sha((source/name).read_bytes()) for name in [test,script]},harnessFiles=hashes,semanticFallbacks=fallbacks,native=native,admitted=False)
    if a.command=='native':
        report['engineSha256']=sha(a.engine.read_bytes())
        wanted=[r for r in baseline['observations'] if r['test'] in plugin.cases]
        report['observationsMatch']=wanted==plugin.observations
        if not report['observationsMatch'] or fallbacks:code=1
    report['exitCode']=code;write_json(out,report);print(dict(cases=len(plugin.cases),exitCode=code));raise SystemExit(code)


if __name__=='__main__':main()
