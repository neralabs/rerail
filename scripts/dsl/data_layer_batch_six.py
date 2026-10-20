# SPDX-License-Identifier: Apache-2.0
"""Observe the sixth batch's unchanged source analyzers and per-check tests."""
from __future__ import annotations

import ast
from dataclasses import asdict, is_dataclass
from functools import wraps
import importlib
import inspect
import json
from pathlib import Path
import re
import time
import sys
from weakref import WeakKeyDictionary

BATCH_SIX = (
    'serverplan_bypass', 'serverplan_steps', 'serverplan_state_tripwires',
    'serverplan_dispatch_architecture', 'serverplan_quality', 'serverplan_target_architecture',
    'execution_authority_bypass', 'impl_direct_execution', 'raw_execute_regression', 'legacy_execution_surfaces',
)
MODULES = {name:'scripts.lint_'+name for name in BATCH_SIX}
TESTS = {name:'scripts/tests/test_lint_'+name+'.py' for name in BATCH_SIX}
TESTS['conditional_architecture_facts']='scripts/tests/test_conditional_architecture_facts.py'


def normalize(check, result, root, files):
    if result is None:return []
    if hasattr(result,'rules'):
        result=[v for rule in result.rules.values() for v in rule.violations]
    if isinstance(result,dict):result=result.get('violations',[])
    output=[]
    for item in result:
        if isinstance(item,str):
            match=re.match(r'^(.*?):(\d+):\s*(.*)$',item)
            if match:path,line,message=match.groups()
            elif ': contains banned raw-surface text ' in item:path,message=item.split(': ',1);line=1
            else:raise ValueError('Unrecognized textual finding: '+item)
            value={'path':path,'line':int(line),'message':message}
        else:value=asdict(item) if is_dataclass(item) else vars(item) if not isinstance(item,dict) else item
        path=Path(value.get('path',value.get('file',value.get('file_path',files[0]['path'] if files else 'input.py'))))
        if path.is_absolute():
            path=path.resolve()
            path=path.relative_to(root) if root and path.is_relative_to(root) else path.relative_to(Path('/'))
        message=value.get('message',value.get('reason',''))
        code=value.get('code',value.get('rule'))
        if check=='serverplan_bypass':code=value.get('method_name',value.get('method','bypass'))
        if check=='impl_direct_execution':code='IDE001';value['line']=value.get('line_num',1)
        if check=='raw_execute_regression':
            code='removed_definition' if message.startswith('definition of ') else 'removed_call' if message.startswith('call to ') else 'mixed_helper_text'
        output.append({'path':path.as_posix(),'line':int(value.get('line',value.get('line_number',1))),
                       'column':None,'code':code,'message':message})
    return output


def install(recorder):
    trees=WeakKeyDictionary()
    original_parse=ast.parse
    @wraps(original_parse)
    def parsed(source,*args,**kwargs):
        tree=original_parse(source,*args,**kwargs)
        if recorder.item is not None and isinstance(source,(str,bytes)):
            text=source.decode('utf-8') if isinstance(source,bytes) else source
            for node in ast.walk(tree):trees[node]=text
        return tree
    recorder.originals.append((ast,'parse',original_parse));ast.parse=parsed
    modules={name:importlib.import_module(path) for name,path in MODULES.items()}
    for name,module in modules.items():sys.modules.setdefault(MODULES[name].removeprefix('scripts.'),module)
    def snapshot(module,entry,arguments):
        pairs=arguments.get('file_sources')
        if pairs is not None:
            root=Path(getattr(module,'ROOT',recorder.root)).resolve()
            def logical(p):
                path=Path(p).resolve() if Path(p).is_absolute() else Path(p)
                return path.relative_to(root).as_posix() if path.is_absolute() and path.is_relative_to(root) else path.relative_to('/').as_posix() if path.is_absolute() else path.as_posix()
            return [{'path':logical(p),'source':source} for p,source in pairs],root
        view=arguments.get('view')
        if view is not None:
            root=Path(view.root).resolve()
            paths=[Path(p) if Path(p).is_absolute() else root / p for p in view.files]
            return [{'path':p.relative_to(root).as_posix(),'source':p.read_text()} for p in paths if p.is_file()],root
        sources=arguments.get('sources')
        if isinstance(sources,dict) and all(isinstance(v,str) for v in sources.values()):
            return [{'path':Path(p).as_posix(),'source':v} for p,v in sources.items()],None
        parsed_modules=arguments.get('parsed_modules')
        if parsed_modules is not None:
            root=Path(getattr(module,'ROOT',recorder.root)).resolve()
            def original_source(p,t):
                if t in trees:return trees[t]
                path=Path(p) if Path(p).is_absolute() else root / p
                source=path.read_text()
                if ast.dump(original_parse(source))!=ast.dump(t):raise ValueError('Cached AST differs from original file text')
                return source
            def logical(p):
                path=Path(p).resolve() if Path(p).is_absolute() else Path(p)
                return path.relative_to(root).as_posix() if path.is_absolute() and path.is_relative_to(root) else path.relative_to('/').as_posix() if path.is_absolute() else path.as_posix()
            return [{'path':logical(p),'source':original_source(p,t)} for p,t in parsed_modules],root
        supplied=next((arguments[k] for k in ('file_path','filepath','file','path','rel_path') if k in arguments),None)
        source=next((arguments[k] for k in ('source','content','text') if isinstance(arguments.get(k),str)),None)
        tree=arguments.get('tree')
        if source is None and isinstance(tree,ast.AST):source=trees.get(tree)
        if supplied is None and source is not None:supplied='input.py'
        if supplied is None:raise ValueError('No source-bearing analyzer argument')
        path=Path(supplied)
        if path.is_absolute():path=path.resolve()
        candidates=[Path(getattr(module,'ROOT',recorder.root)).resolve(),recorder.root,recorder.item.funcargs.get('tmp_path')]
        root=next((Path(candidate).resolve() for candidate in candidates if candidate and path.is_absolute() and path.is_relative_to(Path(candidate).resolve())),None)
        if source is None and path.is_file():source=path.read_text()
        if source is None:raise ValueError('No original source text for AST/file input')
        logical=path.relative_to(root) if root and path.is_absolute() and path.is_relative_to(root) else path.relative_to('/') if path.is_absolute() else path
        return [{'path':logical.as_posix(),'source':source}],root
    def wrap(original,check,entry,module):
        signature=inspect.signature(original)
        @wraps(original)
        def observed(*args,**kwargs):
            if recorder.item is None or recorder.depth:return original(*args,**kwargs)
            arguments=dict(signature.bind_partial(*args,**kwargs).arguments)
            event={'test':recorder.item.nodeid,'check':check,'entry':entry,
                'call':1+sum(e['test']==recorder.item.nodeid for e in recorder.events),
                'scope':{'include':['**/*.py'],'exclude':[]},'production_scope':False,
                'oracle_config':{'options':{'entryPoint':entry}}}
            try:files,root=snapshot(module,entry,arguments);event['files']=files
            except Exception as error:
                files=[];root=None;event['retained_reason']='Input adapter: '+type(error).__name__+': '+str(error)
            if not Path.read_text.__module__.startswith('pathlib'):event['retained_reason']='Test replaces the source reader; retain upstream I/O assertions'
            if check=='legacy_execution_surfaces':event['scope']['include']=['**/*']
            if check=='serverplan_steps':event['oracle_config']['options']['strictModulesOnly']=arguments.get('strict_modules_only',True)
            if check=='serverplan_state_tripwires':
                event['oracle_config']['options']['sourceProvided']=entry!='scan_tree' or arguments.get('source') is not None
                value=arguments.get('project_return_kinds') or {}
                event['oracle_config']['options']['returnKindsHex']=json.dumps({name:sorted(kinds) for name,kinds in value.items()},sort_keys=True,separators=(',',':')).encode().hex()
            if check=='raw_execute_regression':event['oracle_config']['options']['compatibilityStubFiles']=sorted(module.COMPATIBILITY_STUB_FILES)
            if check=='serverplan_target_architecture':event['oracle_config']['options']['requiredShape']=kwargs.get('include_required_shape',True)
            results=arguments.get('results');before=[]
            if results is not None and hasattr(results,'rules'):before=[id(v) for r in results.rules.values() for v in r.violations]
            recorder.depth+=1;started=time.perf_counter()
            try:result=original(*args,**kwargs);raw=result
            finally:recorder.depth-=1
            event['oracle_ms']=(time.perf_counter()-started)*1000
            if not event.get('retained_reason'):
                if raw is None and results is not None:raw=[v for r in results.rules.values() for v in r.violations if id(v) not in before]
                try:event['findings']=normalize(check,raw,root,files)
                except Exception as error:event['retained_reason']='Output adapter: '+type(error).__name__+': '+str(error)
                if entry=='conditional_architecture_findings':
                    for finding in event.get('findings',[]):finding['code']='SPQ-029'
            recorder.events.append(event)
            return result
        return observed
    entries={
        'serverplan_bypass':['scan_file','scan_source'],
        'serverplan_state_tripwires':['scan_source','scan_tree','scan_project_sources'],
        'serverplan_dispatch_architecture':['scan_source'],
        'serverplan_target_architecture':['scan_sources'],
        'execution_authority_bypass':['scan_source'],
        'impl_direct_execution':['scan_file','scan_source','scan_fts_manager_direct_execution','scan_fts_manager_source'],
        'raw_execute_regression':['scan_file_content','_check_text','_check_ast'],
        'legacy_execution_surfaces':['scan_file_content','scan_sql_post_fetch_filter_content'],
    }
    for check in ('serverplan_steps','serverplan_quality'):
        entries[check]=[name for name,value in vars(modules[check]).items() if inspect.isfunction(value) and
            (name.startswith('check_') if check=='serverplan_steps' else name.startswith('_check_'))]
        entries[check].append('_scan_source' if check=='serverplan_steps' else '_check_serverplan_quality_source')
        if check=='serverplan_steps':entries[check].extend(['_scan_file','scan_legacy_postop_violations'])
    for check,names in entries.items():
        module=modules[check]
        for entry in dict.fromkeys(names):
            original=getattr(module,entry);recorder.originals.append((module,entry,original));setattr(module,entry,wrap(original,check,entry,module))
    shared=importlib.import_module('scripts.conditional_architecture_facts')
    original=shared.conditional_architecture_findings
    wrapped=wrap(original,'serverplan_quality','conditional_architecture_findings',shared)
    for module in (shared,modules['serverplan_quality']):
        recorder.originals.append((module,'conditional_architecture_findings',getattr(module,'conditional_architecture_findings')))
        module.conditional_architecture_findings=wrapped
