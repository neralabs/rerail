"""Run unchanged dispatch assertions and record complete source/native scan observations."""
import argparse
import importlib
import json
from pathlib import Path
import select
import subprocess
import sys
import pytest
from migration import ROOT,PIN,git,sha,read_json,write_json,compare
from dto import canonical


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path,required=name in ['source','out'])
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1)
    a=p.parse_args()
    if not 0<=a.part<a.parts:p.error('Invalid shard')
    source=a.source.absolute();out=a.out.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    test='scripts/tests/test_lint_serverplan_dispatch_architecture.py';script='scripts/lint_serverplan_dispatch_architecture.py'
    for name in [test,script]:compare((source/name).read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+name]))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    from data_layer_audit.native_guard import codes
    original_codes=codes({'serverplan_dispatch_architecture':module});native=[];fallbacks=[];transport=[]
    worker=None
    if a.command=='native':
        from dispatch_port import install
        # Start the controlled transport before upstream installs its process audit
        # hooks. Each request remains restricted to explicit temporary input files.
        worker=subprocess.Popen([sys.executable,str(Path(__file__).with_name('replay.py')),
            '--workspace-worker' if a.mode=='workspace' else '--worker',str(a.engine.absolute())],
            cwd=ROOT,text=True,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        class Backend:
            def __init__(self):self.cache={}
            def replace(self,owner,name,value):setattr(owner,name,value)
            def native(self,check,entry,files,options=None):
                transport.append(dict(entry=entry,files=files,options=options))
                if entry=='scan_source':
                    compare(len(files),1)
                    key=(options['dispatchPath'],files[0]['source'])
                    if key not in self.cache:raise AssertionError('Original assertion submitted an unrecorded scan input')
                    return self.cache[key]
                return self.execute(entry,files,options)['diagnostics']
            def execute(self,entry,files,options=None):
                request=dict(rule=str(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr'),
                    files=files,options=dict(entryPoint=entry,**(options or {})))
                worker.stdin.write(json.dumps(request)+'\n');worker.stdin.flush()
                if not select.select([worker.stdout],[],[],110)[0]:
                    worker.kill();raise RuntimeError('Native dispatch assertion exceeded 110 seconds')
                line=worker.stdout.readline()
                if not line:raise RuntimeError('Native dispatch worker exited: '+worker.stderr.read())
                reply=json.loads(line)
                if 'error' in reply:raise RuntimeError(reply['error'])
                compare(reply['executionMode'],a.mode);r=reply['result']
                native.append(r);return r
            def prefetch(self,observations):
                # Only recorded inputs enter this request. Oracle return values
                # never enter the native engine or the response cache.
                files=[dict(path=f'original-{i}.py',source=r['source']) for i,r in enumerate(observations)]
                if not files:return
                r=self.execute('scan_source',files,dict(dispatchDocuments=[f['path'] for f in files],dispatchPaths=[r['path'] for r in observations]))
                groups={f['path']:[] for f in files}
                for diagnostic in r['diagnostics']:groups[diagnostic['parameters']['document']].append(diagnostic)
                for i,row in enumerate(observations):
                    key=(row['path'],row['source'])
                    value=[dict(d,parameters={k:v for k,v in d['parameters'].items() if k!='document'}) for d in groups[f'original-{i}.py']]
                    if key in self.cache:compare(self.cache[key],value)
                    self.cache[key]=value
        baseline=read_json(a.baseline)
        compare(baseline['exitCode'],0)
        compare(baseline['sourcePin'],PIN)
        compare(baseline['sourceFiles'],{name:sha((source/name).read_bytes()) for name in [test,script]})
        backend=Backend();selected=set(baseline['collected'][a.part::a.parts])
        backend.prefetch([r for r in baseline['observations'] if r['test'] in selected])
        install(backend,module,detectors=True)
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
    files=[Path(__file__),*[Path(__file__).with_name(n) for n in ['dispatch_port.py','dispatch_reporter_port.py','python_value_transport.py','fact_transport.py','migration.py','replay.py','dto.py']]]
    hashes={f.relative_to(ROOT).as_posix():sha(f.read_bytes()) for f in files}
    try:code=int(pytest.main(['--rootdir',str(source),'-c',str(source/'pyproject.toml'),'-q',str(source/test)],plugins=[plugin]))
    finally:
        if worker:
            worker.stdin.close();worker.wait(timeout=10)
    report=dict(format='rerail.dispatch-original/v1',sourcePin=PIN,oracle=a.command=='source',mode=a.mode,exitCode=code,
        part=a.part,parts=a.parts,collected=plugin.collected,cases=plugin.cases,observations=plugin.observations,
        sourceFiles={name:sha((source/name).read_bytes()) for name in [test,script]},harnessFiles=hashes,semanticFallbacks=fallbacks,
        native=native,transportCalls=transport,admitted=False)
    if a.command=='native':
        compare(plugin.collected,baseline['collected'])
        report['engineSha256']=sha(a.engine.read_bytes())
        wanted=[r for r in baseline['observations'] if r['test'] in plugin.cases]
        report['observationsMatch']=wanted==plugin.observations
        if not report['observationsMatch'] or fallbacks:code=1
    report['exitCode']=code;write_json(out,report);print(dict(cases=len(plugin.cases),exitCode=code));raise SystemExit(code)


if __name__=='__main__':main()
