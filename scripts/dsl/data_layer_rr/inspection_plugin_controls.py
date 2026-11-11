"""Require native plugin contracts to reject lost paths, extraction, and reporting."""
import argparse
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile

from dto import canonical
from inspection_plugin_contract import exercise
from inspection_port import install
from migration import ROOT,MANIFEST,PIN,read_json,write_json,sha,git,compare
from replay import execute_native,execute_workspace
from validate_supplied_facts_checkpoint import equal


def run(source,engine,baseline,out):
    source=source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_no_source_inspection')
    from data_layer_audit.native_guard import codes
    guarded=codes({'no_source_inspection':module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in guarded and not guarded[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
    oracle=read_json(baseline);compare(oracle['passed'],True)
    cases={c['input']['id']:c for c in oracle['cases']}
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/no-source-inspection.rr'
    library=ROOT/'docs/rule-syntax/lib/data-layer/supplied-facts.rr'
    api=ROOT/'docs/rule-syntax/lib/data-layer/inspection-api.rr'
    edits={
        'caller-path':('fallback-bundle-path',library,
            library.read_text().replace('sf_field(b,r,k,p) :- sf_row(b,r), sf_source_bundle(b,p), sf_config("path_field",k), !sf_property(b,r,k,_).\n','').replace(' !sf_source_bundle(b,_),','')),
        'registered-extraction':('registered-extraction',api,
            api.read_text().replace(', inspection_owner(d,p,owner).',', inspection_owner(d,p,owner), 1 = 0.')),
        'supplied-report':('fallback-bundle-path',None,None),
    }
    records=[];cache=tempfile.TemporaryDirectory(prefix='inspection-plugin-control-cache-')
    os.environ['RERAIL_TAPE_CACHE']=str(Path(cache.name)/'store.sqlite')
    for name,(identity,lib,text) in edits.items():
        for mode,execute in [('snapshot',execute_native),('workspace',execute_workspace)]:
            with tempfile.TemporaryDirectory(prefix='inspection-plugin-control-',dir=ROOT/'.rerail') as directory:
                directory=Path(directory);mutated=policy.read_text()
                if lib:
                    compare(text!=lib.read_text(),True)
                    changed=directory/('control-'+lib.name);changed.write_text(text)
                    include='#include "'+lib.relative_to(ROOT/'docs/rule-syntax/lib').as_posix()+'"'
                    compare(mutated.count(include),1)
                    mutated=mutated.replace(include,'#include '+json.dumps(changed.name))
                else:mutated+='\nsf_bad(b,r) :- sf_row(b,r).\n'
                target=directory/'policy.rr';target.write_text(mutated)
                class Backend:
                    def __init__(self):self.calls=[]
                    def replace(self,owner,key,value):setattr(owner,key,value)
                    def native(self,check,entry,files,options=None):
                        compare(check,'no_source_inspection')
                        result=execute(engine,target,files,dict(entryPoint=entry,**(options or {})))
                        self.calls.append(result);return result['diagnostics']
                backend=Backend();install(backend,module,detectors=True)
                row=cases[identity]
                sys.setprofile(guard)
                try:
                    with tempfile.TemporaryDirectory(prefix='inspection-plugin-control-view-') as scratch:
                        actual=exercise(module,row['input'],Path(scratch))
                finally:sys.setprofile(None)
                if equal(actual,row['expected']):raise AssertionError('Ineffective native control: '+name)
                records.append(dict(control=name,mode=mode,input=row['input'],expected=row['expected'],actual=actual,
                    rejected=True,native=backend.calls,mutatedPolicy=mutated,mutatedLibrary=text,
                    policySha256=sha(mutated.encode()),librarySha256=sha(text.encode()) if text else None))
    if fallbacks:raise AssertionError('Control executed original semantics')
    value=dict(format='rerail.inspection-plugin-controls/v1',passed=True,fullMigrationAcceptance=False,
        engineSha256=sha(engine.read_bytes()),baselineSha256=sha(baseline.read_bytes()),semanticFallbacks=fallbacks,cases=records,
        harnessSha256=sha(Path(__file__).read_bytes()))
    write_json(out,value);print(dict(cases=len(records),passed=True))


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for name in ['source','engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();run(args.source,args.engine.absolute(),args.baseline,args.out)
