"""Run shared MCP tests with complete API observations and native fallback guards."""
import argparse
import importlib
import os
from pathlib import Path
import subprocess
import sys
import pytest
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from mcp_observations import ENTRIES,Observer,validate


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','original-baseline','out']:parser.add_argument('--'+name,type=Path,required=name in {'source','out'})
    parser.add_argument('--checks',default=','.join(ENTRIES));parser.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot')
    parser.add_argument('--test',action='append');args=parser.parse_args()
    for name in ['source','engine','baseline','original_baseline','out']:
        value=getattr(args,name)
        if value is not None:setattr(args,name,value.absolute())
    source=args.source
    compare(git(source,'rev-parse','HEAD'),PIN);sys.path[:0]=[str(source),str(source/'scripts')]
    test='tests/l9875_mcp/test_mcp_architecture_contracts.py'
    targets=[test+'::'+name for name in args.test] if args.test else [test]
    modules={check:importlib.import_module('scripts.lint_'+check) for check in ENTRIES}
    paths=[source/test,*[source/f'scripts/lint_{check}.py' for check in ENTRIES]]
    for path in paths:compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+path.relative_to(source).as_posix()]))
    source_hashes={p.relative_to(source).as_posix():sha(p.read_bytes()) for p in paths}
    dependencies=[Path(__file__),Path(__file__).with_name('mcp_observations.py'),Path(__file__).with_name('dto.py')]
    before={p:sha(p.read_bytes()) for p in (ROOT/'scripts/dsl').rglob('*.py')}
    observer=Observer(modules);backend=None
    if args.command=='native':
        from suite import Suite
        backend=Suite(source,args.engine.absolute(),args.checks.split(','),0,1,args.original_baseline.absolute(),execution_mode=args.mode)
        baseline=read_json(args.baseline);compare(baseline['sourcePin'],PIN);compare(baseline['exitCode'],0);compare(baseline['sourceFiles'],source_hashes)
    class Plugin:
        def __init__(self):self.cases={};self.collected=[]
        def pytest_configure(self):
            if backend:backend.pytest_configure()
            observer.install()
        def pytest_collection_modifyitems(self,items):
            self.collected=[item.nodeid for item in items]
            if backend:backend.pytest_collection_modifyitems(items)
        @pytest.hookimpl(tryfirst=True)
        def pytest_runtest_setup(self,item):
            observer.item=item
            if backend:backend.pytest_runtest_setup(item)
        def pytest_runtest_logreport(self,report):
            self.cases.setdefault(report.nodeid,[]).append(dict(phase=report.when,outcome=report.outcome))
            if backend:backend.pytest_runtest_logreport(report)
            if report.when=='teardown':observer.item=None
        def pytest_unconfigure(self):
            observer.restore()
            if backend:backend.pytest_unconfigure()
    plugin=Plugin();os.chdir(source)
    code=int(pytest.main(['--rootdir',str(source),'-c',str(source/'pyproject.toml'),'-q',*targets],plugins=[plugin]))
    loaded={p for p in dependencies}
    for module in tuple(sys.modules.values()):
        filename=getattr(module,'__file__',None)
        if filename:
            p=Path(filename).absolute()
            if p in before:loaded.add(p)
    hashes={p.relative_to(ROOT).as_posix():before[p] for p in loaded}
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    value=dict(format='rerail.mcp-observations/v1',sourcePin=PIN,oracle=args.command=='source',mode=args.mode,
        exitCode=code,sourceFiles=source_hashes,harnessFiles=hashes,collected=plugin.collected,cases=plugin.cases,
        observations=observer.rows,fullMigrationAcceptance=False)
    if backend:
        wanted=[r for r in baseline['observations'] if r['test'] in plugin.collected]
        try:validate(wanted,observer.rows)
        except ValueError as error:value['comparisonError']=str(error);code=1
        value.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()),
            calls=backend.calls,semanticFallbacks=backend.semantic_fallbacks)
        if backend.semantic_fallbacks:code=1
    value['exitCode']=code;write_json(args.out,value)
    print(dict(cases=len(plugin.cases),observations=len(observer.rows),exitCode=code));raise SystemExit(code)


if __name__=='__main__':main()
