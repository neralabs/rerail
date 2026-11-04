#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Record or compare complete upstream scanner and discovery API results."""
import argparse
import ast
from functools import wraps
import importlib
import json
import os
from pathlib import Path
import sys

import pytest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from data_layer_batch_eight import BATCH_EIGHT,MODULES,TESTS
from data_layer_probe_record import verify_sources
from facts_parity import differences


class Oracle:
    def __init__(self,root):self.root=root;self.item=None;self.originals=[];self.outcomes={}
    def replace(self,module,name,value):
        self.originals.append((module,name,getattr(module,name)));setattr(module,name,value)
    def pytest_configure(self):
        self.modules={check:importlib.import_module(name) for check,name in MODULES.items()}
        from api_observations import install
        install(self)
    @pytest.hookimpl(tryfirst=True)
    def pytest_runtest_setup(self,item):self.item=item
    def pytest_runtest_logreport(self,report):
        self.outcomes.setdefault(report.nodeid,[]).append(dict(phase=report.when,outcome=report.outcome))
        if report.when=='teardown':self.item=None
    def pytest_unconfigure(self):
        for module,name,value in reversed(self.originals):setattr(module,name,value)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('action',choices=['record','compare'])
    for name in ('source','repo','output'):p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--oracle',type=Path);p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot')
    p.add_argument('tests',nargs='*');a=p.parse_args();root=a.source.resolve();workspace=Path.cwd();out=a.output.resolve();oracle=a.oracle.resolve() if a.oracle else None
    sys.path[:0]=[str(root),str(root/'scripts'),str(Path(__file__).parent)];os.chdir(root)
    tests=a.tests or [name for check in BATCH_EIGHT for name in TESTS[check] if (root/name).exists()]
    if a.action=='record':backend=Oracle(root)
    else:
        from native_suite import NativeSuite
        class Compared(NativeSuite):
            def pytest_configure(self):
                super().pytest_configure()
                from api_observations import install
                install(self)
        backend=Compared(root,workspace,a.mode,out)
    code=pytest.main(['--rootdir',str(root),'-c',str(root/'pyproject.toml'),'-q',*[str(root/name) for name in tests]],plugins=[backend])
    if getattr(backend,'semantic_fallbacks',[]):code=pytest.ExitCode.TESTS_FAILED
    report=dict(format='rerail.complete-api-observations/v1',exitCode=int(code),mode='oracle' if a.action=='record' else a.mode,
      sourceFilesChecked=verify_sources(root,a.repo.resolve(),'2b1504adc4291daaf080b2e932962b134da1a69a'),observations=backend.api_observations)
    if a.action=='compare':
        from native_suite import receipt
        report['semanticFallbacks']=backend.semantic_fallbacks
        assertions=receipt(backend,code,workspace/'packs/data-layer/candidates/rules',report['sourceFilesChecked'])
        out.with_suffix('.assertions.json').write_text(json.dumps(assertions,indent=2)+'\n')
    if oracle:
        expected=json.loads(oracle.read_text())['observations']
        def keyed(rows):
            result={};counts={}
            for row in rows:
                key=(row['test'],row['check'],row['entry']);n=counts.get(key,0);counts[key]=n+1
                result['::'.join((*key,str(n)))]=row
            return result
        report['differences']=differences(keyed(expected),keyed(backend.api_observations))
    out.parent.mkdir(parents=True,exist_ok=True);out.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(exitCode=int(code),observations=len(backend.api_observations),differences=len(report.get('differences',[])))))
    if code or report.get('differences'):raise SystemExit(1)


if __name__=='__main__':main()
