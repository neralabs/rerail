"""Remove native executor behavior and require source-recorded parity failures."""
import argparse
import importlib
import os
from pathlib import Path
import sys
import tempfile
from executor_port import CHECK,install
from classvar_production_port import production
from executor_public_contract import exercise as public_exercise
from executor_components_contract import exercise as component_exercise
from executor_production_contract import exercise as production_exercise
from executor_cli_contract import exercise as cli_exercise
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from replay import execute_native,execute_workspace,execute_repository
from validate_supplied_facts_checkpoint import equal

EDITS={
    'scan':('public','check_source-0','main','disable','ppe_Executor(p,l,l,0,0,clean,',None),
    'suppression':('public','check_source-6','main','negation','ppe_Executor(p,l,l,0,0,clean,',None),
    'strict-path':('public','should_check_file-0','main','disable','executor_strict(p) :- executor_path(p), suffix(".py", p), contains("impl/sqlite", p).',None),
    'allowed-path':('public','is_allowed-12','main','disable','executor_allowed(p) :- executor_path(p), match("(?s:.*)(?:impl/sqlite/native_async_wrapper',None),
    'read-warning':('public','missing','api','replace','Warning: Could not read ','Changed warning: '),
    'extractor-metadata':('components','metadata','main','replace','async-executor-v1','disabled-extractor-version'),
    'supplied-report':('components','report-7','api','append',None,'\nsf_excluded(b) :- sf_bundle(b,_).\n'),
    'strict-line':('components','report-9','api','replace','sf_config("strict_line","true").','sf_config("strict_line","false").'),
    'strict-source':('components','report-10','api','disable-fact','sf_strict("source").',None),
    'supplied-key':('components','report-7','api','replace','sf_path_input(b,r,p) :- sf_active(b,r),sf_source_bundle(b,p).','sf_path_input(b,r,"changed.py") :- sf_active(b,r),sf_source_bundle(b,p).'),
    'preview':('components','report-10','api','replace','@slice(s,0,80)','@slice(s,0,1)'),
    'report-order':('components','report-9','api','replace','sf_config("order","bundle").','sf_config("order","line").'),
    'report-strict-scope':('components','scope-2','api','disable','sf_excluded(b) :- sf_source_bundle(b,p),!executor_strict(p).',None),
    'report-allowed-scope':('components','scope-1','api','disable','sf_excluded(b) :- sf_source_bundle(b,p),executor_allowed(p).',None),
    'alias-scope':('production','alias_allowed','main','disable','pp_report_excluded(p) :- pp_absolute(p,full), executor_allowed(full).',None),
    'production-tests':('production','nested','main','replace','pp_config("include_tests","false").','pp_config("include_tests","true").'),
    'production-root':('production','roots','main','replace','pp_static_root("impl").','pp_static_root("outside").'),
    'severity':('production','duplicate','main','replace','pp_config("findingSeverity","error").','pp_config("findingSeverity","warning").'),
    'cli-status':('cli','positive','cli','replace','ExecutorCliExit(@place("",0),1) :-','ExecutorCliExit(@place("",0),0) :-'),
    'cli-count':('cli','callback-positive','cli','replace','exc_active(f,_,"strict"),exc_line(f,r,_)','exc_active(f,_,"strict"),exc_line(f,_,_)'),
    'cli-allowed-group':('cli','allowed-default-root','cli','disable','exc_allowed(i) :-',None),
    'cli-allowed-display':('cli','allowed-strict-default-root','cli','disable','exc_show("allowed") :-',None),
    'cli-explicit-root':('cli','non-strict-explicit-root','cli','replace','(option("execExplicitRoot","1");executor_strict(p))','executor_strict(p)'),
    'cli-verbose':('cli','verbose','cli','disable','ExecutorVerbose(@place(',None),
}
EXERCISES=dict(public=public_exercise,components=component_exercise,production=production_exercise,cli=cli_exercise)


def mutate(text,kind,before,after):
    if kind=='append':return text+after
    if kind in {'negation','disable-fact'}:
        lines=[line for line in text.splitlines(keepends=True) if line.startswith(before)];compare(len(lines),1)
        if kind=='negation':
            compare(lines[0].count('!match('),1);after=lines[0].replace('!match(','match(',1)
        else:after=lines[0].replace(before,before[:-1]+' :- 1=0.',1)
        before=lines[0]
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
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/async-executor.rr'
    libraries={name:ROOT/f'docs/rule-syntax/lib/data-layer/executor-{name}.rr' for name in ['api','cli']}
    paths={Path(__file__).resolve()}
    for owner in tuple(sys.modules.values()):
        filename=getattr(owner,'__file__',None)
        if filename:
            path=Path(filename).resolve()
            if path.suffix=='.py' and path.is_relative_to(Path(__file__).resolve().parent.parent):paths.add(path)
    report=dict(format='rerail.executor-controls/v1',sourcePin=PIN,passed=False,complete=False,part=a.part,parts=a.parts,engineSha256=sha(a.engine.read_bytes()),
        baselineSha256={g:sha((a.work/(g+'-source-accepted.json')).read_bytes()) for g in baselines},harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths},semanticFallbacks=fallbacks,cases=[])
    with tempfile.TemporaryDirectory(prefix='executor-control-cache-') as cache:
        os.environ['RERAIL_TAPE_CACHE']=str(Path(cache)/'cache.sqlite')
        for i,(name,(group,identity,area,kind,before,after)) in enumerate(EDITS.items()):
            if i%a.parts!=a.part:continue
            for mode,execute in [('snapshot',execute_native),('workspace',execute_workspace)]:
                with tempfile.TemporaryDirectory(prefix='executor-control-',dir=ROOT/'.rerail') as temporary:
                    folder=Path(temporary);text=policy.read_text();extra=[]
                    if area=='main':text=mutate(text,kind,before,after)
                    else:
                        changed=mutate(libraries[area].read_text(),kind,before,after);extra.append(changed);(folder/'control.rr').write_text(changed);text=text.replace(f'#include "data-layer/executor-{area}.rr"','#include "control.rr"')
                    target=folder/'policy.rr';target.write_text(text)
                    class Backend:
                        def __init__(self):self.calls=[];self.replaced=[]
                        def replace(self,owner,key,value):self.replaced.append((owner,key,getattr(owner,key)));setattr(owner,key,value)
                        def native(self,check,entry,files,options=None,*,root=None):
                            compare(check,CHECK);options=dict(({} if entry is None else dict(entryPoint=entry)),**(options or {}))
                            result=execute(a.engine,target,files,options) if root is None else execute_repository(a.engine,target,root,options,mode)
                            self.calls.append(result);return result['diagnostics']
                    backend=Backend();install(backend,module,detectors=True);module._rerail_executor_production=lambda root:production(backend,root,check=CHECK)
                    row=oracle[group][identity];sys.setprofile(guard)
                    try:
                        with tempfile.TemporaryDirectory(prefix='executor-control-input-') as inputs:actual=EXERCISES[group](module,row['input'],Path(inputs))
                    finally:
                        sys.setprofile(None)
                        for owner,key,value in reversed(backend.replaced):setattr(owner,key,value)
                        del module._rerail_executor_production
                    rejected=not equal(actual,row['expected']);report['cases'].append(dict(control=name,group=group,mode=mode,input=row['input'],expected=row['expected'],actual=actual,rejected=rejected,native=backend.calls,programCopies={sha(t.encode()):t for t in [text,*extra]}));write_json(a.out,report)
                    if not rejected:raise AssertionError('Ineffective native control: '+name)
                    if isinstance(actual,dict) and 'error' in actual:raise AssertionError('Invalid native mutant: '+str(actual))
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    compare(fallbacks,[]);report.update(passed=True,complete=True);write_json(a.out,report);print(dict(cases=len(report['cases']),passed=True))


if __name__=='__main__':main()
