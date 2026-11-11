"""Remove native type-safety behavior and require source-recorded parity failures."""
import argparse
import importlib
import os
from pathlib import Path
import sys
import tempfile
from types_port import CHECK,install
from classvar_production_port import production
from types_public_contract import exercise as public_exercise
from types_components_contract import exercise as component_exercise
from types_production_contract import exercise as production_exercise
from types_cli_contract import exercise as cli_exercise
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from replay import execute_native,execute_workspace,execute_repository
from validate_supplied_facts_checkpoint import equal

CATEGORIES=['STR_UNION_EXPRESSION','STR_UNION_FIELD','EXPRESSION_ANY_ANY','DEPRECATED_COMPARISON_OP','DEPRECATED_FILTER_METHOD']
EDITS={**{'regex-'+str(i):('production','all_categories','main','disable',f'tsp_issue(d, owner, l, "{c}", 0, {i},',None) for i,c in enumerate(CATEGORIES)},
    **{'signature-'+name:('production','all_categories','main','disable',f'tsp_signature_match(f, owner, "STR_IN_{name.upper()}",',None) for name in ['order_by','where','select','group_by']},
    'callback':('components','callback-order','api','disable','TspCallback(@place(',None),
    'live-allowlist':('public','mutable-allowlist','main','disable','tsp_allowed(c,p) :- option("tspAllowOverride","1"),',None),
    'dedupe':('components','dedupe-boolean-integer','main','disable','tsp_duplicate(i) :-',None),
    'strict-load':('components','load-2','api','disable','tsp_load_valid(c,p,l,col,s,m) :-',None),
    'report':('components','report-7','api','append',None,'\nsf_excluded(b) :- sf_bundle(b,_).\n'),
    'projection':('components','project-0','api','replace','TspProjected(@place("",0),c,p,l,col,s,m) :-','TspProjected(@place("",0),c,"disabled",l,col,s,m) :-'),
    'foreign-report-line':('components','foreign-line-field','api','replace','k!="line_number",k!="line".','k!="line_number".'),
    'foreign-load-line':('components','load-foreign-line-only','api','replace','k!="line_number",k!="line".','k!="line_number".'),
    'alias-scope':('production','alias_allowlist','main','replace','tsp_owned(d,p,p) :- pp_doc(d,p),!tsp_api_path(d,_,_).','tsp_owned(d,owner,p) :- pp_doc(d,owner),pp_doc(d,p),!tsp_api_path(d,_,_).'),
    'cli-status':('cli','positive-check','cli','replace','TspCliExit(@place("",0),1) :-','TspCliExit(@place("",0),0) :-'),
    'cli-summary':('cli','summary','cli','replace','!option("tspSummary","1"),','1=1,'),
    'metadata':('components','metadata-0','main','line','tsp_config("signature_triggers",','tsp_config("signature_triggers","[]").\n'),
    'prepare':('cli','positive','api','disable','tsp_prepare_gate() :- option("tspPrepareEntry","_issues_for_files").',None),
    'production-scope':('production','roots','main','replace','pp_config("scope","production").','pp_config("scope","static").'),
}
EXERCISES=dict(public=public_exercise,components=component_exercise,production=production_exercise,cli=cli_exercise)


def mutate(text,kind,before,after):
    if kind=='append':return text+after
    if kind in {'disable','line'}:
        lines=[line for line in text.splitlines(keepends=True) if line.startswith(before)];compare(len(lines),1)
        if kind=='disable':
            compare(lines[0].count(' :- '),1);after=lines[0].replace(' :- ',' :- 1=0,',1)
        before=lines[0]
    compare(text.count(before),1);return text.replace(before,after)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['source','engine','work','out']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args()
    if not 0<=a.part<a.parts:p.error('Invalid control shard')
    a.engine=a.engine.resolve();source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+CHECK)
    from data_layer_audit.native_guard import codes
    originals=codes({CHECK:module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in originals and not originals[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantic execution')
    baselines={g:read_json(a.work/(g+'-source-accepted.json')) for g in EXERCISES}
    for value in baselines.values():compare(value['passed'],True);compare(value['sourcePin'],PIN)
    oracle={g:{r['input']['id']:r for r in value['cases']} for g,value in baselines.items()}
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/type-safety-patterns.rr'
    libraries={name:ROOT/f'docs/rule-syntax/lib/data-layer/type-safety-{name}.rr' for name in ['api','cli']}
    paths={Path(__file__).resolve()}
    for owner in tuple(sys.modules.values()):
        filename=getattr(owner,'__file__',None)
        if filename:
            path=Path(filename).resolve()
            if path.suffix=='.py' and path.is_relative_to(Path(__file__).resolve().parent.parent):paths.add(path)
    report=dict(format='rerail.type-safety-controls/v1',sourcePin=PIN,passed=False,complete=False,part=a.part,parts=a.parts,engineSha256=sha(a.engine.read_bytes()),
        baselineSha256={g:sha((a.work/(g+'-source-accepted.json')).read_bytes()) for g in baselines},harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths},semanticFallbacks=fallbacks,cases=[])
    with tempfile.TemporaryDirectory(prefix='type-safety-control-cache-') as cache:
        os.environ['RERAIL_TAPE_CACHE']=str(Path(cache)/'cache.sqlite')
        for i,(name,(group,identity,area,kind,before,after)) in enumerate(EDITS.items()):
            if i%a.parts!=a.part:continue
            for mode,execute in [('snapshot',execute_native),('workspace',execute_workspace)]:
                with tempfile.TemporaryDirectory(prefix='type-safety-control-',dir=ROOT/'.rerail') as temporary:
                    folder=Path(temporary);text=policy.read_text();extra=[]
                    if area=='main':text=mutate(text,kind,before,after)
                    else:
                        changed=mutate(libraries[area].read_text(),kind,before,after);extra.append(changed);(folder/'control.rr').write_text(changed);text=text.replace(f'#include "data-layer/type-safety-{area}.rr"','#include "control.rr"')
                    target=folder/'policy.rr';target.write_text(text)
                    class Backend:
                        def __init__(self):self.calls=[];self.replaced=[]
                        def replace(self,owner,key,value):self.replaced.append((owner,key,getattr(owner,key)));setattr(owner,key,value)
                        def native(self,check,entry,files,options=None,*,root=None):
                            compare(check,CHECK);options=dict(({} if entry is None else dict(entryPoint=entry)),**(options or {}))
                            result=execute(a.engine,target,files,options) if root is None else execute_repository(a.engine,target,root,options,mode)
                            self.calls.append(result);return result['diagnostics']
                    backend=Backend();install(backend,module,detectors=True);module._rerail_types_production=lambda root:production(backend,root,check=CHECK)
                    row=oracle[group][identity];sys.setprofile(guard)
                    try:
                        with tempfile.TemporaryDirectory(prefix='type-safety-control-input-') as inputs:actual=EXERCISES[group](module,row['input'],Path(inputs))
                    finally:
                        sys.setprofile(None)
                        for owner,key,value in reversed(backend.replaced):setattr(owner,key,value)
                        del module._rerail_types_production
                    rejected=not equal(actual,row['expected']);report['cases'].append(dict(control=name,group=group,mode=mode,input=row['input'],expected=row['expected'],actual=actual,rejected=rejected,native=backend.calls,programCopies={sha(t.encode()):t for t in [text,*extra]}));write_json(a.out,report)
                    if not rejected:raise AssertionError('Ineffective native control: '+name)
                    if isinstance(actual,dict) and 'error' in actual:raise AssertionError('Invalid native mutant: '+str(actual))
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    compare(fallbacks,[]);report.update(passed=True,complete=True);write_json(a.out,report);print(dict(cases=len(report['cases']),passed=True))


if __name__=='__main__':main()
