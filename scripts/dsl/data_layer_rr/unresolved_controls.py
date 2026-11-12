"""Require source-recorded parity failures after removing unresolved-reference behavior."""
import argparse
import importlib
import os
from pathlib import Path
import sys
import tempfile
from migration import ROOT,PIN,git,read_json,write_json,sha,compare
from replay import execute_native,execute_workspace,execute_repository
from policy_routes import policy_filename
from unresolved_port import CHECK,install
from unresolved_components_contract import exercise as components_exercise
from unresolved_public_contract import exercise as public_exercise
from unresolved_cli_contract import exercise as cli_exercise
from unresolved_production_contract import exercise as production_exercise,install as production_install
from validate_supplied_facts_checkpoint import equal

EDITS={
    'inspection-calls':('components','extract-positive','main','uf_config("TYPE_INSPECTION_FUNCTIONS", name).','uf_config("RESOLVED_PATTERNS", name).'),
    'field-access':('components','extract-positive','main','uf_attr(n, obj, "annotation"), uf_name(obj, name)','uf_attr(n, obj, "not_annotation"), uf_name(obj, name)'),
    'model-fields':('components','extract-model_fields','main','uf_subscript_value(obj, owner), uf_attr(owner, _, "model_fields")','uf_subscript_value(obj, owner), uf_attr(owner, _, "not_fields")'),
    'none-safety':('public','none-anywhere','main','!uf_safe_none(parent), ',''),
    'detection-safety':('components','extract-safe_detection','main','!uf_safe_detection(parent), ',''),
    'resolved-fallback':('public','known-fallback','main','!uf_fallback(n),',''),
    'function-resolution':('public','nested-assignment-contributes','main','uf_call(c, "get_type_hints"),','uf_call(c, "not_type_hints"),'),
    'duplicate-events':('public','two-same-line','main','visit_index(n, ordinal)','ordinal=0'),
    'extractor-metadata':('components','metadata','main','unresolved-forward-refs-v1','unresolved-forward-refs-v0'),
    'raw-report-order':('components','report-order','api','sf_config("order","message").','sf_config("order","input").'),
    'raw-default-function':('components','report-6','api','sf_default("function","<module>").','sf_default("function","changed").'),
    'cli-output':('cli','positive','cli','potential unresolved forward reference issues:','changed unresolved forward reference issues:'),
    'cli-status':('cli','positive-check','cli','UnresolvedCliExit(@place("",0),1,1)','UnresolvedCliExit(@place("",0),0,1)'),
    'production-root':('production','roots','main','pp_static_root("api").','pp_static_root("outside").'),
    'production-trigger':('production','roots','main','pp_trigger(".annotation").','pp_trigger(".not_annotation").'),
}
EXERCISES=dict(components=components_exercise,public=public_exercise,cli=cli_exercise,production=production_exercise)
SOURCES={g:g+'-source.json'for g in EXERCISES}


def library_path(area):return 'docs/rule-syntax/lib/data-layer/unresolved-'+area+'.rr'


def invalid_result(actual):
    if not isinstance(actual,dict):return False
    if 'error'in actual or 'unexpected'in actual:return True
    value=actual.get('value')
    return isinstance(value,dict)and 'error'in value and value['error'].get('type')!='SystemExit'


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['source','engine','work','out']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--mode',choices=['snapshot','workspace'],required=True);p.add_argument('--part',type=int,required=True);a=p.parse_args()
    if not 0<=a.part<len(EDITS):p.error('Invalid mutation')
    a.engine=a.engine.resolve();source=a.source.resolve();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+CHECK)
    from data_layer_audit.native_guard import codes
    originals=codes({CHECK:module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call'and frame.f_code in originals and not originals[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original unresolved semantics executed')
    baselines={g:read_json(a.work/file)for g,file in SOURCES.items()}
    for baseline in baselines.values():compare(baseline['passed'],True);compare(baseline['sourcePin'],PIN)
    name,(group,identity,area,before,after)=list(EDITS.items())[a.part];row=next(r for r in baselines[group]['cases']if r['input']['id']==identity)
    directory=ROOT/'gate/data-layer/packs/data-layer/candidates/rr';original=directory/('unresolved-forward-refs-cli.rr'if area=='cli'else'unresolved-forward-refs.rr')
    text=original.read_text();extra=[]
    with tempfile.TemporaryDirectory(prefix='unresolved-control-',dir=ROOT/'.rerail')as temporary:
        folder=Path(temporary);os.environ['RERAIL_TAPE_CACHE']=str(folder/'cache.sqlite')
        if area=='main':
            compare(text.count(before),1);text=text.replace(before,after,1)
        else:
            library=(ROOT/library_path(area)).read_text();compare(library.count(before),1);library=library.replace(before,after,1)
            (folder/'control.rr').write_text(library);extra.append(library);text=text.replace(f'#include "data-layer/unresolved-{area}.rr"','#include "control.rr"')
        policy=folder/'policy.rr';policy.write_text(text);execute=execute_native if a.mode=='snapshot'else execute_workspace
        class Backend:
            def __init__(self):self.calls=[]
            def replace(self,owner,key,value):setattr(owner,key,value)
            def native(self,check,entry,files,options=None,*,root=None):
                compare(check,CHECK);selected=directory/policy_filename(check,entry)
                if selected==original:selected=policy
                options=dict(({}if entry is None else dict(entryPoint=entry)),**(options or {}))
                result=execute(a.engine,selected,files,options)if root is None else execute_repository(a.engine,selected,root,options,a.mode)
                self.calls.append(result);return result['diagnostics']
        backend=Backend();install(backend,module,detectors=True);production_install(backend,module)
        sys.setprofile(guard)
        try:
            with tempfile.TemporaryDirectory()as scratch:actual=EXERCISES[group](module,row['input'],Path(scratch))
        finally:sys.setprofile(None)
        rejected=not equal(actual,row['expected'])
        invalid=invalid_result(actual)
        harness={}
        for owner in tuple(sys.modules.values()):
            filename=getattr(owner,'__file__',None)
            if filename:
                path=Path(filename).resolve()
                if path.suffix=='.py'and path.is_relative_to(ROOT/'scripts'):harness[path.relative_to(ROOT).as_posix()]=sha(path.read_bytes())
        report=dict(format='rerail.unresolved-controls/v1',sourcePin=PIN,engineSha256=sha(a.engine.read_bytes()),passed=rejected and not invalid and not fallbacks,complete=True,
            baselineSha256={g:sha((a.work/file).read_bytes())for g,file in SOURCES.items()},mode=a.mode,part=a.part,control=name,group=group,input=row['input'],
            expected=row['expected'],actual=actual,rejected=rejected,semanticFallbacks=fallbacks,native=backend.calls,harnessFiles=harness,programCopies={sha(t.encode()):t for t in [text,*extra]})
        write_json(a.out,report);print(dict(control=name,rejected=rejected,invalid=invalid));raise SystemExit(0 if report['passed']else 1)


if __name__=='__main__':main()
