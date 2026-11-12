"""Remove native admin behavior and require source-recorded parity failures."""
import argparse
import importlib
import os
from pathlib import Path
import sys
import tempfile
from admin_components_port import CHECK,install
from admin_components_contract import exercise as components_exercise
from admin_public_contract import exercise as public_exercise
from admin_cli_contract import exercise as cli_exercise
from admin_production_contract import exercise as production_exercise
from admin_production_port import production
from executor_controls import mutate as common_mutate
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from replay import execute_native,execute_workspace,execute_repository
from validate_supplied_facts_checkpoint import equal

EDITS={
    **{f'pattern-{i}':('components','extract-all','main','disable-head',f'admin_candidate(d, p, l, t, {i},',None) for i in range(3)},
    'quote-state':('production','docstring','main','replace','parity % 2 = 0','parity % 2 = 1'),
    'owner-skip':('production','alias_skip','main','all','!admin_skip_path(p), ',''),
    'allowlist':('components','extract-allowlisted','main','all','!admin_allowlisted(p), ',''),
    'metadata':('components','metadata','main','replace','admin-async-purity-v1','disabled-extractor-version'),
    'severity':('components','report-9','main','replace','pp_config("findingSeverity","error").','pp_config("findingSeverity","hard-fail").'),
    'report-int':('components','report-41','api','disable','ar_line(b,r,value) :-',None),
    'report-int-error':('components','report-43','api','disable','ar_error(b,r,3,error_type,detail) :-',None),
    'report-message':('components','report-9','api','disable','sf_message(b,r,cat(pattern,',None),
    'report-order':('components','report-lexical-line-order','api','replace','sf_order(b,r,cat(path,":",line,":",message),0,0,"","")','sf_order(b,r,to_string(r),0,0,"","")'),
    'project':('components','project-0','api','replace','",\\"line\\":",l,','",\\"line\\":","999",'),
    'cli-header':('cli','positive','main','replace','async purity violation(s) in admin tools:','Changed violation heading:'),
    'cli-check':('cli','positive-check','cli','replace','AdminCliExit(@place("",0),1) :- ap_cli_count(n)','AdminCliExit(@place("",0),0) :- ap_cli_count(n)'),
    'cli-signed':('cli','sign-callback-True','cli','replace','AdminCliExit(@place("",0),0) :- option("entryPoint","_ap_cli_signed")','AdminCliExit(@place("",0),1) :- option("entryPoint","_ap_cli_signed")'),
    'production-root':('production','roots','main','replace','pp_static_root("l8000_admin/tools").','pp_static_root("outside").'),
    'production-prefilter':('production','roots','main','all','pp_trigger("','pp_trigger("disabled-'),
}
EXERCISES=dict(components=components_exercise,public=public_exercise,cli=cli_exercise,production=production_exercise)
SOURCES={g:g+'-source.json' for g in EXERCISES}


def mutate(text,kind,before,after):
    if kind=='all':
        if before not in text:raise ValueError('Missing control text')
        return text.replace(before,after)
    if kind=='disable-head':
        lines=text.splitlines(keepends=True);matched=[i for i,line in enumerate(lines) if line.startswith(before)]
        if not matched:raise ValueError('Missing control head: '+before)
        for i in matched:
            compare(lines[i].count(' :- '),1);lines[i]=lines[i].replace(' :- ',' :- 1=0,',1)
        return ''.join(lines)
    return common_mutate(text,kind,before,after)


def library_path(area):return f'docs/rule-syntax/lib/data-layer/admin-{area}.rr'


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
    extra={name:importlib.import_module(name) for name in ['lint_lib.signing','scripts.lint_lib.signing']}
    from data_layer_audit.native_guard import codes
    originals=codes({CHECK:module,**extra});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in originals and not originals[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantic execution')
    baselines={g:read_json(a.work/file) for g,file in SOURCES.items()}
    for value in baselines.values():compare(value['passed'],True);compare(value['sourcePin'],PIN)
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/admin-async-purity.rr'
    name,(group,identity,area,kind,before,after)=list(EDITS.items())[a.part]
    row=next(r for r in baselines[group]['cases'] if r['input']['id']==identity)
    report=dict(format='rerail.admin-controls/v1',sourcePin=PIN,passed=False,complete=False,mode=a.mode,part=a.part,
        engineSha256=sha(a.engine.read_bytes()),baselineSha256={g:sha((a.work/file).read_bytes()) for g,file in SOURCES.items()},semanticFallbacks=fallbacks,cases=[])
    with tempfile.TemporaryDirectory(prefix='admin-control-cache-') as cache:
        os.environ['RERAIL_TAPE_CACHE']=str(Path(cache)/'cache.sqlite')
        with tempfile.TemporaryDirectory(prefix='admin-control-',dir=ROOT/'.rerail') as temporary:
            folder=Path(temporary);text=policy.read_text();extra_text=[]
            if area=='main':text=mutate(text,kind,before,after)
            else:
                changed=mutate((ROOT/library_path(area)).read_text(),kind,before,after);extra_text.append(changed)
                (folder/'control.rr').write_text(changed);text=text.replace(f'#include "data-layer/admin-{area}.rr"','#include "control.rr"')
            target=folder/'policy.rr';target.write_text(text)
            execute=execute_native if a.mode=='snapshot' else execute_workspace
            class Backend:
                def __init__(self):self.calls=[];self.replaced=[]
                def replace(self,owner,key,value):self.replaced.append((owner,key,getattr(owner,key)));setattr(owner,key,value)
                def native(self,check,entry,files,options=None,*,root=None):
                    compare(check,CHECK);options=dict(({} if entry is None else dict(entryPoint=entry)),**(options or {}))
                    result=execute(a.engine,target,files,options) if root is None else execute_repository(a.engine,target,root,options,a.mode)
                    self.calls.append(result);return result['diagnostics']
            backend=Backend();install(backend,module,detectors=True);module._rerail_admin_production=lambda root:production(backend,root)
            report['harnessFiles']=harness_files()
            sys.setprofile(guard)
            try:
                with tempfile.TemporaryDirectory(prefix='admin-control-input-') as inputs:actual=EXERCISES[group](module,row['input'],Path(inputs))
            finally:
                sys.setprofile(None)
                for owner,key,value in reversed(backend.replaced):setattr(owner,key,value)
                del module._rerail_admin_production
            rejected=not equal(actual,row['expected'])
            report['cases'].append(dict(control=name,group=group,mode=a.mode,input=row['input'],expected=row['expected'],actual=actual,rejected=rejected,native=backend.calls,programCopies={sha(t.encode()):t for t in [text,*extra_text]}))
    for filename,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/filename).read_bytes()))
    report['harnessFiles'].update(harness_files())
    write_json(a.out,report)
    if not rejected:raise AssertionError('Ineffective native control: '+name)
    if invalid_result(actual):raise AssertionError('Invalid native mutant: '+str(actual))
    compare(fallbacks,[]);report.update(passed=True,complete=True);write_json(a.out,report);print(dict(control=name,mode=a.mode,passed=True))


if __name__=='__main__':main()

