"""Require observed parity failures after removing native connection-union behavior."""
import argparse
import importlib
import os
from pathlib import Path
import sys
import tempfile
from conn_union_port import CHECK,install
from conn_union_contract import exercise as scan_exercise
from conn_union_components_contract import exercise as component_exercise
from conn_union_production_contract import exercise as production_exercise
from conn_union_cli_contract import exercise as cli_exercise
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from replay import execute_native,execute_workspace,execute_repository
from validate_supplied_facts_checkpoint import equal

EDITS={
    'union':('scanner','annotation-union','main','cut_first_union(f, _, name, ann), annotation_text','1=0, cut_first_union(f, _, name, ann), annotation_text'),
    'separate':('scanner','shape-separate','main','cut_issue(f, "SEPARATE_PARAMS", cat("Has both connection (", connections, ") and transaction (", transactions, ") params")) :-','cut_issue(f, "SEPARATE_PARAMS", cat("Has both connection (", connections, ") and transaction (", transactions, ") params")) :- 1=0,'),
    'allowlist':('scanner','shape-qualified-allow','main','cut_allowed(f) :-','cut_allowed(f) :- 1=0,'),
    'argument-precedence':('scanner','shape-argument-order','main','r = min rank','r = max rank'),
    'type-helper':('components','helper-_has_connection_and_transaction-5','api','cut_graph_kind(n,"BinOp"),','1=0,cut_graph_kind(n,"BinOp"),'),
    'supplied-facts':('components','report-6','api','sf_path_input(b,r,p) :-','sf_path_input(b,r,p) :- 1=0,'),
    'field-types':('components','report-9','api','sf_strict("function_name").',''),
    'signature':('components','report-signature-0','api','option("entryPoint","_report_facts"),option("cutOwnerRoot",root),!allow_status','1=0,option("entryPoint","_report_facts"),option("cutOwnerRoot",root),!allow_status'),
    'production-selection':('production','positive','production','cut_root(p) :- !env','cut_root(p) :- p!="impl",!env'),
    'production-facts':('production','positive','production','UnionProductionFact(@place("",0),p,@replace_all(p,"/","\\u0000"),i,l,q,c,m) :-','UnionProductionFact(@place("",0),p,@replace_all(p,"/","\\u0000"),i,l,q,c,m) :- 1=0,'),
    'cli-status':('cli','positive-check','cli','UnionCliExit(@place("",0),1) :- cut_cli_count','UnionCliExit(@place("",0),0) :- cut_cli_count'),
    'cli-file-selection':('cli','suffixes','cli',',!prefix("test_",n),',','),
}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['source','engine','work','out']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    if not 0<=args.part<args.parts:parser.error('Invalid control shard')
    args.engine=args.engine.resolve()
    if 'NERA_SIGNING_KEY' in os.environ:raise AssertionError('Run this worker with env -u NERA_SIGNING_KEY')
    source=args.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+CHECK)
    signing=importlib.import_module('scripts.lint_lib.signing')
    from data_layer_audit.native_guard import codes
    originals=codes({CHECK:module,'signing':signing});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in originals and not originals[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
    baselines={name:read_json(args.work/file) for name,file in [('scanner','scanner-source.json'),('components','components-source-expanded.json'),('production','production-source-current.json'),('cli','cli-source-current.json')]}
    for value in baselines.values():compare(value['passed'],True);compare(value['sourcePin'],PIN)
    oracle={k:{r['input']['id']:r for r in v['cases']} for k,v in baselines.items()}
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/conn-or-txn-union.rr'
    libraries={name:ROOT/f'docs/rule-syntax/lib/data-layer/conn-union-{name}.rr' for name in ['api','production','cli']}
    paths={Path(__file__).resolve()}
    for owner in tuple(sys.modules.values()):
        name=getattr(owner,'__file__',None)
        if name:
            path=Path(name).resolve()
            if path.suffix=='.py' and path.is_relative_to(Path(__file__).resolve().parent.parent):paths.add(path)
    report=dict(format='rerail.conn-union-controls/v1',sourcePin=PIN,passed=False,complete=False,part=args.part,parts=args.parts,
        engineSha256=sha(args.engine.read_bytes()),baselineSha256={k:sha((args.work/file).read_bytes()) for k,file in [('scanner','scanner-source.json'),('components','components-source-expanded.json'),('production','production-source-current.json'),('cli','cli-source-current.json')]},
        harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths},semanticFallbacks=fallbacks,cases=[])
    with tempfile.TemporaryDirectory(prefix='conn-union-control-cache-') as cache:
        os.environ['RERAIL_TAPE_CACHE']=str(Path(cache)/'cache.sqlite')
        for index,(name,(group,identity,area,before,after)) in enumerate(EDITS.items()):
            if index%args.parts!=args.part:continue
            for mode,execute in [('snapshot',execute_native),('workspace',execute_workspace)]:
                with tempfile.TemporaryDirectory(prefix='conn-union-control-',dir=ROOT/'.rerail') as temporary:
                    folder=Path(temporary);main=policy.read_text();extra=[]
                    if area=='main':compare(main.count(before),1);main=main.replace(before,after)
                    else:
                        text=libraries[area].read_text();compare(text.count(before),1);text=text.replace(before,after);extra.append(text)
                        (folder/'control.rr').write_text(text);main=main.replace(f'#include "data-layer/conn-union-{area}.rr"','#include "control.rr"')
                    target=folder/'policy.rr';target.write_text(main)
                    class Backend:
                        def __init__(self):self.calls=[];self.replaced=[]
                        def replace(self,owner,key,value):self.replaced.append((owner,key,getattr(owner,key)));setattr(owner,key,value)
                        def native(self,check,entry,files,options=None,*,root=None):
                            compare(check,CHECK);options=dict(({} if entry is None else dict(entryPoint=entry)),**(options or {}))
                            result=execute(args.engine.absolute(),target,files,options) if root is None else execute_repository(args.engine.absolute(),target,root,options,mode)
                            self.calls.append(result);return result['diagnostics']
                    backend=Backend();install(backend,module,detectors=True)
                    module._rerail_union_production=lambda root:__import__('conn_union_port').production(backend,module,root)
                    row=oracle[group][identity];exercise={'scanner':scan_exercise,'components':component_exercise,'production':production_exercise,'cli':cli_exercise}[group]
                    sys.setprofile(guard)
                    try:
                        with tempfile.TemporaryDirectory(prefix='conn-union-control-input-') as inputs:actual=exercise(module,row['input'],Path(inputs))
                    finally:
                        sys.setprofile(None)
                        for owner,key,value in reversed(backend.replaced):setattr(owner,key,value)
                        del module._rerail_union_production
                        del module._rerail_union_extract
                    rejected=not equal(actual,row['expected'])
                    report['cases'].append(dict(control=name,group=group,mode=mode,input=row['input'],expected=row['expected'],actual=actual,rejected=rejected,native=backend.calls,programCopies={sha(t.encode()):t for t in [main,*extra]}));write_json(args.out,report)
                    if not rejected:raise AssertionError('Ineffective native control: '+name)
                    if isinstance(actual,dict) and 'error' in actual:raise AssertionError('Invalid mutant: '+str(actual))
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    compare(fallbacks,[]);report.update(passed=True,complete=True);write_json(args.out,report);print(dict(cases=len(report['cases']),passed=True))


if __name__=='__main__':main()
