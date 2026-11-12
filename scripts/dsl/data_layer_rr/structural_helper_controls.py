"""Require effective native mutations for shared structural helper behavior."""
import argparse
import importlib
import os
from pathlib import Path
import sys
import tempfile
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from replay import execute_native,execute_workspace
from structural_helper_contract import CHECK,exercise
from structural_helper_port import install
from validate_supplied_facts_checkpoint import equal

POLICY='gate/data-layer/packs/data-layer/candidates/rr/connection-structural-boundaries-supplied.rr'
LIBRARY='docs/rule-syntax/lib/data-layer/structural-connection-helper.rr'
EDITS={
    'line-boundary':('lines-above-limit','n>to_number(limit)','n<to_number(limit)'),
    'residual-symbols':('class-WritePlanWorkflow','option_item("schResidualSymbols",_,name)','option_item("schAllowedBases",_,name)'),
    'allowed-bases':('forbidden','!option_item("schAllowedBases",_,name)','option_item("schAllowedBases",_,name)'),
    'attribute-bases':('attribute','sch_kind(n,"Attribute")','sch_kind(n,"Name")'),
    'first-core':('first-core-wins','i=min j:','i=max j:'),
    'duplicate-bases':('duplicates','sch_rank(c,_,name,rank)}','sch_rank(c,_,name,rank),rank=0}'),
}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path,required=True)
    p.add_argument('--mode',choices=['snapshot','workspace'],required=True);p.add_argument('--part',type=int,required=True);a=p.parse_args()
    if not 0<=a.part<len(EDITS):p.error('Invalid control')
    source=a.source.resolve();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+CHECK)
    baseline=read_json(a.baseline);compare(baseline['passed'],True);compare(baseline['sourceSha256'],sha(Path(module.__file__).read_bytes()))
    name,(identity,before,after)=list(EDITS.items())[a.part];row=next(r for r in baseline['cases']if r['input']['id']==identity)
    from data_layer_audit.native_guard import codes
    originals=codes({CHECK:module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call'and frame.f_code in originals and not originals[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original structural semantics executed')
    text=(ROOT/LIBRARY).read_text()
    if text.count(before)!=1:raise ValueError('Control must alter one reviewed clause')
    text=text.replace(before,after,1);main_text=(ROOT/POLICY).read_text().replace('#include "data-layer/structural-connection-helper.rr"','#include "control.rr"')
    native=[]
    with tempfile.TemporaryDirectory(prefix='structural-control-',dir=ROOT/'.rerail')as control:
        control=Path(control);policy=control/'policy.rr';policy.write_text(main_text);(control/'control.rr').write_text(text)
        os.environ['RERAIL_TAPE_CACHE']=str(control/'store.sqlite')
        execute=execute_native if a.mode=='snapshot'else execute_workspace
        class Backend:
            def replace(self,owner,name,value):setattr(owner,name,value)
            def native(self,check,entry,files,options=None):
                result=execute(a.engine.resolve(),policy,files,dict(entryPoint=entry,**(options or {})));native.append(result);return result['diagnostics']
        install(Backend(),module)
        sys.setprofile(guard)
        try:
            with tempfile.TemporaryDirectory()as scratch:actual=exercise(module,row['input'],Path(scratch))
        finally:sys.setprofile(None)
    rejected=not equal(actual,row['expected'])and not any(k in actual for k in ['error','unexpected'])and bool(native)and not fallbacks
    harness={}
    for owner in tuple(sys.modules.values()):
        filename=getattr(owner,'__file__',None)
        if filename:
            path=Path(filename).resolve()
            if path.suffix=='.py'and path.is_relative_to(ROOT/'scripts'):harness[path.relative_to(ROOT).as_posix()]=sha(path.read_bytes())
    write_json(a.out,dict(format='rerail.structural-helper-controls/v1',sourcePin=PIN,engineSha256=sha(a.engine.read_bytes()),
        baselineSha256=sha(a.baseline.read_bytes()),mode=a.mode,part=a.part,control=name,input=row['input'],expected=row['expected'],actual=actual,
        passed=rejected,complete=True,semanticFallbacks=fallbacks,native=native,harnessFiles=harness,
        programCopies={sha(t.encode()):t for t in [main_text,text]}))
    print(dict(control=name,rejected=rejected));raise SystemExit(0 if rejected else 1)


if __name__=='__main__':main()
