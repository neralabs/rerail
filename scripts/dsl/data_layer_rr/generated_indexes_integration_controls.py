"""Remove generated-index integration behavior and require observable parity failures."""
import argparse
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile
from generated_indexes_integration_contract import CHECK,exercise
from generated_indexes_port import install
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from replay import execute_native,execute_workspace
from validate_supplied_facts_checkpoint import equal


def run(source,engine,baseline,out):
    source=source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+CHECK)
    from data_layer_audit.native_guard import codes
    originals=codes({CHECK:module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in originals and not originals[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
    oracle=read_json(baseline);compare(oracle['passed'],True);cases={r['input']['id']:r for r in oracle['cases']}
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/generated-index-sources.rr'
    library=ROOT/'docs/rule-syntax/lib/data-layer/generated-indexes-api.rr'
    table=policy.with_name('generated-index-sources.tables.rr')
    edits={
        'metadata-version':('metadata','table','generated-index-sources-v2','control-version'),
        'supplied-report':('report-7','library',None,None),
        'cli-status':('cli-check-positive','library','GeneratedCliExit(@place("",0),1) :-','GeneratedCliExit(@place("",0),0) :-'),
        'production-selection':('production-positive','library','pp_file(p) :-','pp_file(p) :- 1 = 0,'),
        'production-facts':('production-positive','library','GeneratedProductionFact(@place("",0),p,@replace_all(p,"/","\\u0000"),i,r,0,l,m) :-','GeneratedProductionFact(@place("",0),p,@replace_all(p,"/","\\u0000"),i,r,0,l,m) :- 1 = 0,'),
        'extraction-facts':('extract-positive','library','GeneratedCollectorFact(@place(p,l),i,r,l,m) :-','GeneratedCollectorFact(@place(p,l),i,r,l,m) :- 1 = 0,'),
        'parse-helper':('parse-8','library','gis_parsed(p,@python_int_text(raw),m) :-','gis_parsed(p,@python_int_text(raw),m) :- 1 = 0,'),
        'format-helper':('format-0','library','cat(p,":",l,": ",m)) :-','cat("control:",p,":",l,": ",m)) :-'),
        'unavailable-error':('unavailable-extractor','table','generated_index_sources uses AstEventCollector facts','control extractor error'),
    }
    result=dict(format='rerail.generated-indexes-integration-controls/v1',sourcePin=PIN,passed=False,
        engineSha256=sha(engine.read_bytes()),baselineSha256=sha(baseline.read_bytes()),semanticFallbacks=fallbacks,cases=[])
    with tempfile.TemporaryDirectory(prefix='generated-index-integration-cache-') as cache:
        os.environ['RERAIL_TAPE_CACHE']=str(Path(cache)/'cache.sqlite')
        for name,(identity,area,before,after) in edits.items():
            modified=(table if area=='table' else library).read_text()
            if before is None:modified+='\nsf_excluded(b) :- sf_bundle(b,_).\n'
            else:compare(before in modified,True);modified=modified.replace(before,after)
            for mode,execute in [('snapshot',execute_native),('workspace',execute_workspace)]:
                with tempfile.TemporaryDirectory(prefix='generated-index-integration-control-',dir=ROOT/'.rerail') as temporary:
                    folder=Path(temporary);main_text=policy.read_text();table_text=modified if area=='table' else table.read_text()
                    (folder/table.name).write_text(table_text)
                    if area=='library':
                        (folder/'control-api.rr').write_text(modified)
                        main_text=main_text.replace('#include "data-layer/generated-indexes-api.rr"','#include "control-api.rr"')
                    target=folder/'policy.rr';target.write_text(main_text)
                    class Backend:
                        def __init__(self):self.calls=[];self.replaced=[]
                        def replace(self,owner,key,value):self.replaced.append((owner,key,getattr(owner,key)));setattr(owner,key,value)
                        def native(self,check,entry,files,options=None,*,root=None):
                            compare(check,CHECK);arguments=dict(({} if entry is None else dict(entryPoint=entry)),**(options or {}))
                            if root is None:native=execute(engine,target,files,arguments)
                            else:
                                from replay import execute_repository
                                native=execute_repository(engine,target,root,arguments,mode)
                            self.calls.append(native);return native['diagnostics']
                    backend=Backend();install(backend,module,detectors=True);row=cases[identity]
                    sys.setprofile(guard)
                    try:
                        with tempfile.TemporaryDirectory(prefix='generated-index-integration-input-') as inputs:actual=exercise(module,row['input'],Path(inputs))
                    finally:
                        sys.setprofile(None)
                        for owner,key,value in reversed(backend.replaced):setattr(owner,key,value)
                    rejected=not equal(actual,row['expected'])
                    case=dict(control=name,mode=mode,input=row['input'],expected=row['expected'],actual=actual,rejected=rejected,native=backend.calls,
                        programCopies={sha(t.encode()):t for t in [main_text,table_text,*([modified] if area=='library' else [])]})
                    result['cases'].append(case);write_json(out,result)
                    if not rejected:raise AssertionError('Ineffective integration control: '+name)
                    if 'error' in actual and name!='unavailable-error':raise AssertionError('Invalid integration control: '+str(actual))
    compare(fallbacks,[]);result['passed']=True;write_json(out,result);print(dict(cases=len(result['cases']),passed=True))


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for name in ['source','engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();run(args.source,args.engine.absolute(),args.baseline,args.out)
