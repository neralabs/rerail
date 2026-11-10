"""Run every original shared-ratchet assertion against the native compatibility API."""
import argparse
import contextlib
import importlib
import inspect
import io
import json
import os
from pathlib import Path
import select
import subprocess
import sys
from types import SimpleNamespace

import pytest

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from policy_routes import policy_filename


def normalize(value, root):
    if isinstance(value,Path):value=str(value)
    if isinstance(value,str):return value.replace(str(root),'$ROOT')
    if isinstance(value,dict):return {k:normalize(v,root) for k,v in value.items()}
    if isinstance(value,(tuple,list)):return [normalize(v,root) for v in value]
    return value


class Contract:
    def __init__(self,args):
        self.args=args;self.calls=[];self.cases={};self.collected=[];self.native_calls=[];self.fallbacks=[];self.item=None
        self.module=importlib.import_module('scripts.lint_lib.ratchet')
        self.original=self.module.check_ratchet
        self.codes={f.__code__:n for n,f in vars(self.module).items() if inspect.isfunction(f) and f.__module__==self.module.__name__}
        self.signature=inspect.signature(self.original)
        self.worker=None
        if args.command=='native':
            self.worker=subprocess.Popen([sys.executable,str(Path(__file__).with_name('replay.py')),
                '--workspace-worker' if args.mode=='workspace' else '--worker',str(args.engine)],
                cwd=ROOT,env=dict(os.environ),text=True,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)

    def native(self,check,entry,files=(),options=None):
        request=dict(rule=str(ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/policy_filename(check,entry)),files=list(files),options=dict(entryPoint=entry,**(options or {})))
        self.worker.stdin.write(json.dumps(request)+'\n');self.worker.stdin.flush()
        if not select.select([self.worker.stdout],[],[],110)[0]:
            self.worker.kill();raise RuntimeError('Native shared-ratchet assertion exceeded 110 seconds')
        line=self.worker.stdout.readline()
        if not line:raise RuntimeError('Native worker exited: '+self.worker.stderr.read())
        reply=json.loads(line)
        if 'error' in reply:raise RuntimeError(reply['error'])
        compare(reply['executionMode'],self.args.mode);result=reply['result']
        self.native_calls.append(dict(result,entry=entry));return result['diagnostics']

    def replace(self,owner,name,value):setattr(owner,name,value)

    def pytest_configure(self):
        function=self.original
        if self.args.command=='native':
            from impl_direct_execution_port import install
            settings=self.native('impl_direct_execution','configuration',options=dict(env=['ide.api=true']))
            allowed={r['parameters']['value'] for r in settings if r['parameters']['setting']=='allowed_file'}
            proxy=SimpleNamespace(ALLOWED_FILES=allowed,lint_ratchet=self.module,lint_signing=SimpleNamespace())
            install(self,proxy);function=self.module.check_ratchet
        def wrapped(*args,**kwargs):
            root=self.item.funcargs['tmp_path'];event=dict(test=self.item.nodeid,arguments=normalize(dict(self.signature.bind(*args,**kwargs).arguments),root))
            out,err=io.StringIO(),io.StringIO()
            try:
                with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):result=function(*args,**kwargs)
                event['return']=result;return result
            except BaseException as error:
                event['error']=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=normalize(error.args,root));raise
            finally:
                event.update(stdout=normalize(out.getvalue(),root),stderr=normalize(err.getvalue(),root));self.calls.append(event)
                sys.stdout.write(out.getvalue());sys.stderr.write(err.getvalue())
        self.module.check_ratchet=wrapped
        def guard(frame,event,arg):
            if event=='call' and frame.f_code in self.codes:
                name=self.codes[frame.f_code];self.fallbacks.append(name);raise AssertionError('Original ratchet function invoked: '+name)
        self.guard=guard

    def pytest_collection_modifyitems(self,items):self.collected=[i.nodeid for i in items]
    def pytest_runtest_setup(self,item):
        self.item=item
        if self.args.command=='native':sys.setprofile(self.guard)
    def pytest_runtest_logreport(self,report):self.cases.setdefault(report.nodeid,[]).append(dict(phase=report.when,outcome=report.outcome))
    def pytest_unconfigure(self):
        sys.setprofile(None);self.module.check_ratchet=self.original


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path,required=n in ['source','out'])
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');a=p.parse_args();a.source=a.source.absolute()
    if a.engine:a.engine=a.engine.absolute()
    assert git(a.source,'rev-parse','HEAD')==PIN
    files=['scripts/lint_lib/ratchet.py','scripts/lint_lib/signing.py','scripts/tests/test_ratchet_lib.py']
    for name in files:compare((a.source/name).read_bytes(),subprocess.check_output(['git','-C',str(a.source),'show',PIN+':'+name]))
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['impl_direct_execution_port.py','policy_routes.py','replay.py','migration.py']]]
    harness={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in paths}
    sys.path[:0]=[str(a.source),str(a.source/'scripts')];contract=Contract(a)
    code=pytest.main(['-q','-p','no:cacheprovider','-o','addopts=',str(a.source/'scripts/tests/test_ratchet_lib.py')],plugins=[contract])
    if contract.worker:
        contract.worker.stdin.close();contract.worker.wait(timeout=10)
    report=dict(format='rerail.ratchet-original-contract/v1',sourcePin=PIN,mode=a.command,executionMode=a.mode,
        fullMigrationAcceptance=False,exitCode=int(code),collected=contract.collected,cases=contract.cases,calls=contract.calls,
        native=contract.native_calls,semanticFallbacks=contract.fallbacks,harnessFiles=harness,sourceFiles={n:sha((a.source/n).read_bytes()) for n in files})
    assert len(report['collected'])==3 and not report['semanticFallbacks']
    for name,h in harness.items():compare(h,sha((ROOT/name).read_bytes()))
    if a.command=='native':
        baseline=read_json(a.baseline);assert baseline['exitCode']==0
        for key in ['sourcePin','collected','cases','calls','sourceFiles']:compare(report[key],baseline[key])
        report['engineSha256']=sha(a.engine.read_bytes())
    write_json(a.out,report);raise SystemExit(code)


if __name__=='__main__':main()
