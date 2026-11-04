# SPDX-License-Identifier: Apache-2.0
"""Record unchanged eighth-batch analyzer calls with their complete source domain."""
from __future__ import annotations
import ast
import copy
from dataclasses import asdict, is_dataclass
from functools import wraps
import importlib
import inspect
import json
from pathlib import Path
import time
from weakref import WeakKeyDictionary

BATCH_EIGHT = ('eager_resolve','lazy_architecture','pure_lazy_runtime_purity','cached_pure_lazy_allowlist','lazy_sql_surface','plan_helpers_allowlist','legacy_callback_quarantine','readthrough_authority_architecture','authentication_plan_purity','lazy_adapter_ownership')
MODULES = {name:'scripts.lint_'+name for name in BATCH_EIGHT}
MODULES['cached_pure_lazy_allowlist']='scripts.verify_cached_pure_lazy_allowlist'
TESTS = {name:['scripts/tests/test_lint_'+name+'.py'] for name in BATCH_EIGHT}
TESTS['cached_pure_lazy_allowlist']=['scripts/tests/test_verify_cached_pure_lazy_allowlist.py']
TESTS['lazy_architecture'] += ['scripts/tests/test_lazy_architecture_plugin_parity.py','scripts/tests/test_repository_lazy_architecture.py']


def normalize(check, result, files, logical):
    if isinstance(result,tuple) and len(result)==2 and isinstance(result[1],list):result=result[1]
    if result is None:return []
    findings=[]
    for item in result:
        if isinstance(item,str):
            if check!='cached_pure_lazy_allowlist':raise ValueError('Unexpected textual diagnostic')
            findings.append({'path':'l2055_lazy_result/cached_pure_lazy_policy.py','line':1,'column':None,'code':item,'message':item})
            continue
        value=asdict(item) if is_dataclass(item) else item if isinstance(item,dict) else vars(item)
        path=value.get('file_path',value.get('file',value.get('path',files[0]['path'] if len(files)==1 else None)))
        if path is None:raise ValueError('Finding has no source identity')
        code=value.get('code',value.get('rule',value.get('pattern_name',value.get('reason',value.get('message')))))
        if not code:raise ValueError('Finding has no diagnostic identity')
        findings.append({'path':logical(path),'line':value.get('line_num',value.get('line_number',value.get('line',1))),
                         'column':None,'code':code,'message':value.get('message',value.get('reason',value.get('text','')))})
    return findings


def install(recorder):
    trees=WeakKeyDictionary();original_parse=ast.parse;original_walk=ast.walk
    @wraps(original_parse)
    def parse(source,*args,**kwargs):
        tree=original_parse(source,*args,**kwargs)
        if recorder.item is not None and isinstance(source,(str,bytes)):
            text=source.decode() if isinstance(source,bytes) else source
            for node in original_walk(tree):trees[node]=text
        return tree
    recorder.originals.append((ast,'parse',original_parse));ast.parse=parse
    modules={name:importlib.import_module(module) for name,module in MODULES.items()}
    analyzed_sources={};active_sources=[]
    recorder.source_probes=[]
    cls=modules['lazy_architecture'].FunctionAnalyzer;original_init=cls.__init__
    @wraps(original_init)
    def observed_init(self,path,tree,*args,**kwargs):
        text=trees.get(tree)
        if text is not None:
            analyzed_sources[id(self)]=(path,text)
            if recorder.item is not None:
                root=modules['lazy_architecture'].ROOT
                relative=Path(path).relative_to(root).as_posix() if Path(path).is_relative_to(root) else Path(path).as_posix()
                recorder.source_probes.append({'test':recorder.item.nodeid,'check':'lazy_architecture','files':[{'path':relative,'source':text}]})
            if active_sources:active_sources[-1].append((path,text))
        return original_init(self,path,tree,*args,**kwargs)
    recorder.originals.append((cls,'__init__',original_init));cls.__init__=observed_init
    deleted_observations=[]
    lazy_module=modules['lazy_architecture']
    for name in ('_source_for_deleted_surface_scan','_scan_deleted_cold_bootstrap_surfaces'):
        original=getattr(lazy_module,name)
        def deleted_observer(original,name):
            @wraps(original)
            def observed(*args,**kwargs):
                result=original(*args,**kwargs)
                if deleted_observations:
                    if name=='_source_for_deleted_surface_scan':deleted_observations[-1]['sources'].append((args[0],result))
                    else:deleted_observations[-1]['findings'].extend(result)
                return result
            return observed
        recorder.originals.append((lazy_module,name,original));setattr(lazy_module,name,deleted_observer(original,name))
    discovered_sources={}
    for function_name in ('scan_cached_pure_lazy_source',):
        cached_module=modules['cached_pure_lazy_allowlist']
        original=getattr(cached_module,function_name)
        def source_observer(original):
            @wraps(original)
            def record_source(path,source,*args,**kwargs):
                result=original(path,source,*args,**kwargs)
                for function in result:discovered_sources[id(function)]=(function,path,source)
                return result
            return record_source
        recorder.originals.append((cached_module,function_name,original));setattr(cached_module,function_name,source_observer(original))
    entries={
        'eager_resolve':['_scan_source','_scan_file','_scan_preprocess_ast','_scan_lower_compile_ast','_scan_transitive_eager_ast','_scan_legacy_boundary_definition_ast','_scan_external_io_boundary_definition_ast'],
        'lazy_architecture':['scan_files','_scan_analyzed_files'],
        'pure_lazy_runtime_purity':['scan_source','scan_paths','scan_paths_cached'],
        'cached_pure_lazy_allowlist':['memoized_policy_violations','policy_violations'],
        'lazy_sql_surface':['scan_source','scan_file','scan_repo'],
        'plan_helpers_allowlist':['scan_file_content','scan_file','scan_repo'],
        'legacy_callback_quarantine':['scan_file_content','scan_repo'],
        'readthrough_authority_architecture':['scan_sources'],
        'authentication_plan_purity':['scan_sources'],
        'lazy_adapter_ownership':['scan_source','violations'],
    }
    def wrap(original,check,entry,module):
        signature=inspect.signature(original)
        @wraps(original)
        def observed(*args,**kwargs):
            if recorder.item is None or recorder.depth:return original(*args,**kwargs)
            arguments=dict(signature.bind_partial(*args,**kwargs).arguments)
            event={'test':recorder.item.nodeid,'check':check,'entry':entry,'call':1+sum(e['test']==recorder.item.nodeid for e in recorder.events),
                   'scope':{'include':['**/*.py'],'exclude':[]},'production_scope':False,'oracle_config':{'options':{'oracleAnalyzer':True,'entryPoint':entry}}}
            roots=[arguments.get('root'),recorder.item.funcargs.get('tmp_path'),getattr(module,'ROOT',getattr(module,'REPO_ROOT',recorder.root))]
            roots=[Path(p).resolve() for p in roots if isinstance(p,(str,Path))]
            def logical(path):
                path=Path(path)
                if path.is_absolute():
                    root=next((r for r in roots if path.is_relative_to(r)),None)
                    return path.relative_to(root).as_posix() if root else path.as_posix()
                return path.as_posix()
            files=[]
            try:
                source=arguments.get('source',arguments.get('content'));path=arguments.get('file_path',arguments.get('path',arguments.get('rel_path','input.py')))
                tree=next((v for v in arguments.values() if isinstance(v,ast.AST)),None)
                if source is None and tree is not None:source=ast.unparse(tree) if check=='eager_resolve' else trees.get(tree)
                if check=='eager_resolve':event['oracle_config']['options']['externalIoBoundaries']=module._load_external_io_boundary_allowlist()
                if isinstance(source,str):files=[{'path':logical(path),'source':source}]
                else:
                    paths=arguments.get('paths',arguments.get('files'))
                    if check=='lazy_architecture' and 'by_file' in arguments:
                        files=[{'path':logical(path),'source':text} for analyzer in arguments['by_file'].values() for path,text in [analyzed_sources[id(analyzer)]]];paths=[]
                    if check=='cached_pure_lazy_allowlist':
                        functions=arguments.get('functions',arguments.get('discovered',()))
                        paths=sorted({function.path for function in functions})
                        virtual={logical(path):text for function in functions if id(function) in discovered_sources for _,path,text in [discovered_sources[id(function)]]}
                        files=[{'path':path,'source':text} for path,text in virtual.items()]
                        if any(not function.path.is_file() and id(function) not in discovered_sources for function in functions):raise ValueError('Discovered policy facts have no source input')
                        paths=[path for path in paths if logical(path) not in virtual]
                        options=event['oracle_config']['options']
                        options['policyEntries']=json.dumps([asdict(e) for e in arguments.get('entries',arguments.get('policy_entries',()))],sort_keys=True,default=str)
                    if paths is None:
                        if entry in {'scan_file','_scan_file','scan_file_content'}:paths=[path]
                        elif entry=='scan_sources':paths=arguments['paths']
                        elif 'root' in arguments:paths=sorted(Path(arguments['root']).rglob('*.py'))
                        else:raise ValueError('No source-bearing analyzer domain')
                    if not isinstance(paths,(list,tuple,set,frozenset)):raise ValueError('One-shot source iterator cannot be consumed by observation')
                    if check=='pure_lazy_runtime_purity' and entry in {'scan_paths','scan_paths_cached'}:
                        event['oracle_config']['options']['primaryPaths']=[logical(p) for p in paths]
                        directories=arguments.get('directories')
                        if directories is not None:
                            root=Path(arguments.get('root',module.ROOT))
                            paths=sorted(set(paths)|{p for directory in directories for p in (root/directory).rglob('*.py') if 'tests' not in p.parts})
                    for p in paths:
                        p=Path(p)
                        if p.is_file():files.append({'path':logical(p),'source':p.read_text()})
                if not files and check not in {'lazy_architecture','cached_pure_lazy_allowlist','authentication_plan_purity','readthrough_authority_architecture'}:raise ValueError('No readable analyzer inputs')
                if not Path.read_text.__module__.startswith('pathlib'):raise ValueError('Test replaces the original source reader')
                event['files']=files
            except Exception as error:event['retained_reason']='Input adapter: '+type(error).__name__+': '+str(error)
            captured=[];deleted={'sources':[],'findings':[]}
            if check=='lazy_architecture':active_sources.append(captured);deleted_observations.append(deleted)
            recorder.depth+=1;start=time.perf_counter()
            try:result=original(*args,**kwargs)
            finally:
                recorder.depth-=1;event['oracle_ms']=(time.perf_counter()-start)*1000
                if check=='lazy_architecture':active_sources.pop();deleted_observations.pop()
            if check=='lazy_architecture' and captured:
                by_path={logical(path):text for path,text in captured};files=[{'path':path,'source':text} for path,text in by_path.items()];event['files']=files
            extra=None
            if check=='lazy_architecture' and deleted['sources'] and 'retained_reason' not in event:
                read_sources={logical(path):text for path,text in deleted['sources']}
                if any(file['source']!=read_sources.get(file['path'],file['source']) for file in files):
                    # The original mock supplies two different input representations.
                    # Keep actual producer outputs and coordinates, rather than changing findings.
                    extra=copy.deepcopy(event);extra['entry']='_scan_deleted_cold_bootstrap_surfaces';extra['call']+=1
                    extra['oracle_config']['options']['entryPoint']=extra['entry']
                    extra['files']=[{'path':path,'source':text} for path,text in read_sources.items()]
                    extra['findings']=normalize(check,deleted['findings'],extra['files'],logical)
                    extra['input_adapter_note']='Actual fallback source reads and their producer results, separate from the mocked AST coordinates.'
                    excluded={id(item) for item in deleted['findings']}
                    result_for_recording=[item for item in result[1] if id(item) not in excluded]
                    event['entry']='scan_files.ast-component';event['oracle_config']['options']['entryPoint']=event['entry']
                else:result_for_recording=result
            else:result_for_recording=result
            if 'retained_reason' not in event:
                try:event['findings']=normalize(check,result_for_recording,files,logical)
                except Exception as error:event['retained_reason']='Output adapter: '+type(error).__name__+': '+str(error)
            recorder.events.append(event)
            if extra is not None:recorder.events.append(extra)
            return result
        return observed
    for check,names in entries.items():
        module=modules[check]
        for entry in names:
            original=getattr(module,entry);recorder.originals.append((module,entry,original));setattr(module,entry,wrap(original,check,entry,module))
