"""Require observed parity failures after removing native ForwardRef behavior."""
import argparse
import importlib
import os
from pathlib import Path
import sys
import tempfile
from forwardref_port import CHECK,install
from classvar_production_port import production
from forwardref_contract import exercise as scan_exercise
from forwardref_components_contract import exercise as component_exercise
from forwardref_production_contract import exercise as production_exercise
from forwardref_cli_contract import exercise as cli_exercise
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from replay import execute_native,execute_workspace,execute_repository
from validate_supplied_facts_checkpoint import equal

EDITS={
    'tracking':('scanner','call-0','main','fr_assign(a, f) :-','fr_assign(a, f) :- 1=0,'),
    'guard':('scanner','guard-0','main','fr_checked(c) :-','fr_checked(c) :- 1=0,'),
    'sync-scope':('scanner','shape-async-preserved','main','fr_sync(f) :- kind(f, "function_definition"), !op(f, "async").','fr_sync(f) :- kind(f, "function_definition").'),
    'type-helper':('components','contains-1','api','fr_graph_kind(n,"Attribute"),','1=0,fr_graph_kind(n,"Attribute"),'),
    'fact-projection':('components','project-0','api','ForwardProjection(@place("",0),p,l,c,m,k) :-','ForwardProjection(@place("",0),"disabled",l,c,m,k) :-'),
    'collector':('components','extract-positive','api','fr_bad(n,"callable",c,i),line(n,l).','1=0,fr_bad(n,"callable",c,i),line(n,l).'),
    'typed-line':('components','report-7','api','.input sfe_member(IO=json,filename="reporter-facts.json",path="/bundles/*/rows/*/fields/*/key",projection="python-str")',''),
    'supplied-report':('components','report-7','api',None,None),
    'cli-status':('cli','positive','cli','ForwardCliExit(@place("",0),1) :-','ForwardCliExit(@place("",0),0) :-'),
    'cli-selection':('cli','suffixes','cli','frc_excluded(i) :-','frc_excluded(i) :- 1=0,'),
    'production':('production','three-codes','main','pp_static_root("impl").',''),
    'source-gate':('scanner','content-positive-clean','api','fr_prepare_gate(i) :- fr_prepare(i,_,_),!option("frCallSource",_).','fr_prepare_gate(i) :- fr_prepare(i,_,_).'),
}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['source','engine','work','out']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    if not 0<=args.part<args.parts:parser.error('Invalid control shard')
    args.engine=args.engine.resolve()
    source=args.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+CHECK)
    signing=importlib.import_module('scripts.lint_lib.signing')
    from data_layer_audit.native_guard import codes
    originals=codes({CHECK:module,'signing':signing});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in originals and not originals[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
    baselines={name:read_json(args.work/file) for name,file in [('scanner','public-source-expanded.json'),('components','components-source-final.json'),('production','production-source.json'),('cli','cli-source.json')]}
    for value in baselines.values():compare(value['passed'],True);compare(value['sourcePin'],PIN)
    oracle={k:{r['input']['id']:r for r in v['cases']} for k,v in baselines.items()}
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/forwardref.rr'
    libraries={name:ROOT/f'docs/rule-syntax/lib/data-layer/forwardref-{name}.rr' for name in ['api','production','cli']}
    paths={Path(__file__).resolve()}
    for owner in tuple(sys.modules.values()):
        name=getattr(owner,'__file__',None)
        if name:
            path=Path(name).resolve()
            if path.suffix=='.py' and path.is_relative_to(Path(__file__).resolve().parent.parent):paths.add(path)
    report=dict(format='rerail.forwardref-controls/v1',sourcePin=PIN,passed=False,complete=False,part=args.part,parts=args.parts,
        engineSha256=sha(args.engine.read_bytes()),baselineSha256={k:sha((args.work/file).read_bytes()) for k,file in [('scanner','public-source-expanded.json'),('components','components-source-final.json'),('production','production-source.json'),('cli','cli-source.json')]},
        harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths},semanticFallbacks=fallbacks,cases=[])
    with tempfile.TemporaryDirectory(prefix='forwardref-control-cache-') as cache:
        os.environ['RERAIL_TAPE_CACHE']=str(Path(cache)/'cache.sqlite')
        for index,(name,(group,identity,area,before,after)) in enumerate(EDITS.items()):
            if index%args.parts!=args.part:continue
            for mode,execute in [('snapshot',execute_native),('workspace',execute_workspace)]:
                with tempfile.TemporaryDirectory(prefix='forwardref-control-',dir=ROOT/'.rerail') as temporary:
                    folder=Path(temporary);main=policy.read_text();extra=[]
                    if area=='main':compare(main.count(before),1);main=main.replace(before,after)
                    else:
                        text=libraries[area].read_text()
                        if before is None:text+='\nsf_excluded(b) :- sf_bundle(b,_).\n'
                        else:compare(text.count(before),1);text=text.replace(before,after)
                        extra.append(text)
                        (folder/'control.rr').write_text(text);main=main.replace(f'#include "data-layer/forwardref-{area}.rr"','#include "control.rr"')
                    target=folder/'policy.rr';target.write_text(main)
                    class Backend:
                        def __init__(self):self.calls=[];self.replaced=[]
                        def replace(self,owner,key,value):self.replaced.append((owner,key,getattr(owner,key)));setattr(owner,key,value)
                        def native(self,check,entry,files,options=None,*,root=None):
                            compare(check,CHECK);options=dict(({} if entry is None else dict(entryPoint=entry)),**(options or {}))
                            result=execute(args.engine.absolute(),target,files,options) if root is None else execute_repository(args.engine.absolute(),target,root,options,mode)
                            self.calls.append(result);return result['diagnostics']
                    backend=Backend();install(backend,module,detectors=True)
                    module._rerail_forwardref_production=lambda root:production(backend,root,check=CHECK)
                    row=oracle[group][identity];exercise={'scanner':scan_exercise,'components':component_exercise,'production':production_exercise,'cli':cli_exercise}[group]
                    sys.setprofile(guard)
                    try:
                        with tempfile.TemporaryDirectory(prefix='forwardref-control-input-') as inputs:actual=exercise(module,row['input'],Path(inputs))
                    finally:
                        sys.setprofile(None)
                        for owner,key,value in reversed(backend.replaced):setattr(owner,key,value)
                        del module._rerail_forwardref_production
                    rejected=not equal(actual,row['expected'])
                    report['cases'].append(dict(control=name,group=group,mode=mode,input=row['input'],expected=row['expected'],actual=actual,rejected=rejected,native=backend.calls,programCopies={sha(t.encode()):t for t in [main,*extra]}));write_json(args.out,report)
                    if not rejected:raise AssertionError('Ineffective native control: '+name)
                    if isinstance(actual,dict) and 'error' in actual:raise AssertionError('Invalid mutant: '+str(actual))
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    compare(fallbacks,[]);report.update(passed=True,complete=True);write_json(args.out,report);print(dict(cases=len(report['cases']),passed=True))


if __name__=='__main__':main()
