"""Remove native async-pattern behavior and require source-recorded parity failures."""
import argparse
import importlib
import os
from pathlib import Path
import sys
import tempfile
from async_pattern_port import CHECK,install
from async_pattern_helpers_contract import exercise as helpers_exercise
from async_pattern_components_contract import exercise as components_exercise
from async_pattern_files_contract import exercise as files_exercise
from async_pattern_io_contract import exercise as io_exercise
from async_pattern_cli_contract import exercise as cli_exercise
from async_pattern_production_contract import exercise as production_exercise
from async_pattern_production_port import production
from executor_controls import mutate as common_mutate
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from replay import execute_native,execute_workspace,execute_repository
from validate_supplied_facts_checkpoint import equal

EDITS={
    **{code:('production','multiple_codes' if code=='ASYNC006' else 'all_codes','main','disable-head',f'ap_issue(c, owner, "{code}"',None) for code in ['ASYNC001','ASYNC002','ASYNC004','ASYNC005','ASYNC006','ASYNC007','ASYNC008']},
    'alias-scope':('production','alias_scope','main','replace','ap_fallback(c, owner) :- ap_call_doc(c, d, owner), ap_api(d, owner), ap_in_class(c, cls), ap_abstract(cls, d, owner).','ap_fallback(c, owner) :- ap_call_doc(c, d, owner), ap_api(d, other), ap_in_class(c, cls), ap_abstract(cls, d, other).'),
    'constructor-scope':('helpers','initial-0','helpers','disable','aph_initial_true("in_scope") :-',None),
    'context-padding':('helpers','context-3-2','helpers','replace','4-strlen(to_string(i+1))','3-strlen(to_string(i+1))'),
    'unicode-annotation':('helpers','annotation-10','helpers','disable','aph_contains(n) :- ap_graph_kind(n,"Constant")',None),
    'typed-parameter':('helpers','infer-3','main','disable','ap_txn_name(f, name) :- ap_function(f, _, 1), ap_typed_parameter',None),
    'subscript-receiver':('helpers','receiver-2','main','disable','ap_receiver_root(e, name) :- ap_subscript_value',None),
    'query-chain':('helpers','query-1','helpers','disable','aph_query(n) :- ap_call_expr(n,fn),ap_attr(fn,obj,method)',None),
    'collector-prefilter':('components','collector-ignores-prefilter','api','disable','ap_source_triggered(0) :- option("entryPoint","_ap_ast_collect")',None),
    'supplied-report':('components','report-7','api','disable','sf_message(b,r,m) :-',None),
    'fallback-filter':('components','report-11','api','disable','sf_bad(b,r) :- apv_true(b,r,"is_fallback").',None),
    'root-slash':('components','load-root-2','api','replace','sf_path_input(b,r,cat(root,p))','sf_path_input(b,r,cat(root,"/",p))'),
    'directory-bare-suffix':('files','directory-3','files','replace',',!apf_bare_py(actual).','.'),
    'direct-file-bare-suffix':('files','lint-file-9','main','replace',', !apf_bare_py(actual),',','),
    'cli-actionable-count':('cli','fallback-actionable','cli','replace','apl_totals(n,p,total,fb,total-fb)','apl_totals(n,p,total,fb,total)'),
    'cli-ratchet-check':('cli','ratchet-regression','cli','replace','apl_action("check") :-','apl_action("done") :-'),
    'cli-header':('cli','positive','cli','disable','AsyncPatternCliOutput(@place("",0),0,"",0,0,0,0,0,"Async Pattern Lint',None),
    'production-root':('production','roots','main','replace','pp_static_root("api").','pp_static_root("outside").'),
    'read-excluded-file':('io','directory-excluded-decode','files','replace',',!ap_excluded_path(p).','.'),
    'broken-directory-file':('io','directory-excluded-broken-link','files','replace','apf_directory_candidate(i,p),!apf_directory_skip(i).','apf_candidate(i,p),!apf_directory_skip(i).'),
}
EXERCISES=dict(helpers=helpers_exercise,components=components_exercise,files=files_exercise,cli=cli_exercise,production=production_exercise,io=io_exercise)
SOURCES=dict(helpers='helpers-source.json',components='components/source.json',files='files-final-source.json',cli='cli-source.json',production='production-source.json',io='io-source.json')


def mutate(text,kind,before,after):
    if kind=='disable-head':
        lines=text.splitlines(keepends=True);matched=[i for i,line in enumerate(lines) if line.startswith(before)]
        if not matched:raise ValueError('Missing control head: '+before)
        for i in matched:
            compare(lines[i].count(' :- '),1);lines[i]=lines[i].replace(' :- ',' :- 1=0,',1)
        return ''.join(lines)
    return common_mutate(text,kind,before,after)


def library_path(area):return f'docs/rule-syntax/lib/data-layer/async-pattern-{area}.rr'


def invalid_result(actual):
    return isinstance(actual,dict) and ('error' in actual or isinstance(actual.get('value'),dict) and 'error' in actual['value'])


def harness_files():
    paths={Path(__file__).resolve()}
    for owner in tuple(sys.modules.values()):
        filename=getattr(owner,'__file__',None)
        if filename:
            path=Path(filename).resolve()
            if path.suffix=='.py' and path.is_relative_to(Path(__file__).resolve().parent.parent):paths.add(path)
    return {p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['source','engine','work','out']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--mode',choices=['snapshot','workspace'],required=True)
    p.add_argument('--part',type=int,required=True);a=p.parse_args()
    if not 0<=a.part<len(EDITS):p.error('Invalid control shard')
    a.engine=a.engine.resolve();source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+CHECK)
    extra={name:importlib.import_module(name) for name in ['lint_lib.ratchet','scripts.lint_lib.discovery']}
    from data_layer_audit.native_guard import codes
    originals=codes({CHECK:module,**extra});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in originals and not originals[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantic execution')
    baselines={g:read_json(a.work/file) for g,file in SOURCES.items()}
    for value in baselines.values():compare(value['passed'],True);compare(value['sourcePin'],PIN)
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/async-patterns.rr'
    name,(group,identity,area,kind,before,after)=list(EDITS.items())[a.part]
    row=next(r for r in baselines[group]['cases'] if r['input']['id']==identity)
    report=dict(format='rerail.async-pattern-controls/v1',sourcePin=PIN,passed=False,complete=False,mode=a.mode,part=a.part,
        engineSha256=sha(a.engine.read_bytes()),baselineSha256={g:sha((a.work/file).read_bytes()) for g,file in SOURCES.items()},semanticFallbacks=fallbacks,cases=[])
    with tempfile.TemporaryDirectory(prefix='async-pattern-control-cache-') as cache:
        os.environ['RERAIL_TAPE_CACHE']=str(Path(cache)/'cache.sqlite')
        with tempfile.TemporaryDirectory(prefix='async-pattern-control-',dir=ROOT/'.rerail') as temporary:
            folder=Path(temporary);text=policy.read_text();extra_text=[]
            if area=='main':text=mutate(text,kind,before,after)
            else:
                changed=mutate((ROOT/library_path(area)).read_text(),kind,before,after);extra_text.append(changed)
                (folder/'control.rr').write_text(changed);text=text.replace(f'#include "data-layer/async-pattern-{area}.rr"','#include "control.rr"')
            target=folder/'policy.rr';target.write_text(text)
            execute=execute_native if a.mode=='snapshot' else execute_workspace
            class Backend:
                def __init__(self):self.calls=[];self.replaced=[]
                def replace(self,owner,key,value):self.replaced.append((owner,key,getattr(owner,key)));setattr(owner,key,value)
                def native(self,check,entry,files,options=None,*,root=None):
                    compare(check,CHECK);options=dict(({} if entry is None else dict(entryPoint=entry)),**(options or {}))
                    result=execute(a.engine,target,files,options) if root is None else execute_repository(a.engine,target,root,options,a.mode)
                    self.calls.append(result);return result['diagnostics']
            backend=Backend();install(backend,module,detectors=True);module._rerail_async_patterns_production=lambda root:production(backend,root)
            report['harnessFiles']=harness_files()
            sys.setprofile(guard)
            try:
                with tempfile.TemporaryDirectory(prefix='async-pattern-control-input-') as inputs:actual=EXERCISES[group](module,row['input'],Path(inputs))
            finally:
                sys.setprofile(None)
                for owner,key,value in reversed(backend.replaced):setattr(owner,key,value)
                del module._rerail_async_patterns_production
            rejected=not equal(actual,row['expected'])
            report['cases'].append(dict(control=name,group=group,mode=a.mode,input=row['input'],expected=row['expected'],actual=actual,rejected=rejected,native=backend.calls,programCopies={sha(t.encode()):t for t in [text,*extra_text]}))
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    report['harnessFiles'].update(harness_files())
    write_json(a.out,report)
    if not rejected:raise AssertionError('Ineffective native control: '+name)
    if invalid_result(actual):raise AssertionError('Invalid native mutant: '+str(actual))
    compare(fallbacks,[]);report.update(passed=True,complete=True);write_json(a.out,report);print(dict(control=name,mode=a.mode,passed=True))


if __name__=='__main__':main()
