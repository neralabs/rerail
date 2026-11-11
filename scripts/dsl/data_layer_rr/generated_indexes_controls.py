"""Disable real generated-index rule behavior and require source/native disagreement."""
import argparse
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile
from generated_indexes_contract import CHECK,exercise
from generated_indexes_port import install
from migration import ROOT,PIN,git,compare,read_json,write_json,sha
from replay import execute_native,execute_workspace
from validate_supplied_facts_checkpoint import equal


def run(source,engine,baseline,out):
    source=source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_'+CHECK)
    from data_layer_audit.native_guard import codes
    original_codes=codes({CHECK:module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in original_codes and not original_codes[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
    oracle=read_json(baseline);compare(oracle['passed'],True)
    cases={r['input']['id']:r for r in oracle['cases']}
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/generated-index-sources.rr'
    text=policy.read_text()
    edits={
        'direct-call':('direct:check_file','gis_index_call(c) :-','gis_index_call(c) :- 1 = 0,'),
        'import-alias':('unicode-import:check_file','gis_index_name(d, x, i) :-','gis_index_name(d, x, i) :- 1 = 0,'),
        'first-keyword':('duplicate-keyword:check_file','!gis_later_kw(c,a),',''),
        'unicode-name':('unicode-call:check_file','@nfkc(raw)','raw'),
        'path-fallback':('colon-multiline','gis_owner(d,owner),contains(": ",owner).','gis_owner(d,owner),contains(": ",owner),1 = 0.'),
        'required-parameter':('required:check_file','gis_check(f), !gis_source_param(f, _, _, _).','gis_check(f), !gis_source_param(f, _, _, _),1 = 0.'),
        'finding-order':('multiple:check_file','gis_output(d,p,l,why,i,rank) :-','gis_output(d,p,l,why,i,-rank) :-'),
    }
    records=[]
    with tempfile.TemporaryDirectory(prefix='generated-index-controls-cache-') as cache:
        os.environ['RERAIL_TAPE_CACHE']=str(Path(cache)/'store.sqlite')
        for name,(identity,before,after) in edits.items():
            compare(before in text,True);mutated=text.replace(before,after)
            for mode,execute in [('snapshot',execute_native),('workspace',execute_workspace)]:
                with tempfile.TemporaryDirectory(prefix='generated-index-control-',dir=ROOT/'.rerail') as scratch:
                    target=Path(scratch)/'policy.rr';target.write_text(mutated)
                    companion=(policy.parent/'generated-index-sources.tables.rr').read_text()
                    (Path(scratch)/'generated-index-sources.tables.rr').write_text(companion)
                    class Backend:
                        def __init__(self):self.calls=[];self.originals=[]
                        def replace(self,owner,key,value):self.originals.append((owner,key,getattr(owner,key)));setattr(owner,key,value)
                        def native(self,check,entry,files,options=None):
                            compare(check,CHECK)
                            result=execute(engine,target,files,dict(entryPoint=entry,**(options or {})))
                            self.calls.append(result);return result['diagnostics']
                    backend=Backend();install(backend,module,detectors=True);row=cases[identity]
                    sys.setprofile(guard)
                    try:
                        with tempfile.TemporaryDirectory(prefix='generated-index-control-files-') as files:
                            actual=exercise(module,row['input'],Path(files))
                    finally:
                        sys.setprofile(None)
                        for owner,key,original in reversed(backend.originals):setattr(owner,key,original)
                    records.append(dict(control=name,mode=mode,input=row['input'],expected=row['expected'],actual=actual,
                        rejected=not equal(actual,row['expected']),native=backend.calls,mutatedPolicy=mutated,policySha256=sha(mutated.encode()),companionTable=companion))
                    write_json(out,dict(passed=False,cases=records))
                    if isinstance(actual,dict) and 'error' in actual:raise AssertionError('Invalid native mutation: '+str(actual))
                    if equal(actual,row['expected']):raise AssertionError('Ineffective native mutation: '+name)
    if fallbacks:raise AssertionError('Original semantic fallback')
    result=dict(format='rerail.generated-indexes-controls/v1',sourcePin=PIN,passed=True,
        fullMigrationAcceptance=False,engineSha256=sha(engine.read_bytes()),baselineSha256=sha(baseline.read_bytes()),
        semanticFallbacks=fallbacks,cases=records,harnessSha256=sha(Path(__file__).read_bytes()))
    write_json(out,result);print(dict(cases=len(records),passed=True))


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for name in ['source','engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();run(args.source,args.engine.absolute(),args.baseline,args.out)
