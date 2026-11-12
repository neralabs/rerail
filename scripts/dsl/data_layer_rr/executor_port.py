"""Transfer executor inputs and native rows without source linter execution."""
import argparse
from functools import wraps
import inspect
import json
from pathlib import Path
import re
import sys
from types import SimpleNamespace
from typed_fact_transport import encode_bundles

CHECK='async_executor'
ENTRIES=['check_file','check_source','should_check_file','is_allowed']


def one(rows,code,key):
    values=[r['parameters'][key] for r in rows if r['code']==code]
    if len(values)!=1:raise ValueError('One native '+code+' required')
    return values[0]


def query(backend,entry,files,options=None,*,root=None):
    return backend.native(CHECK,entry,files,{'apiInputDirs':['__executor_no_primary__'],**(options or {})},root=root)


def warnings(backend,paths,errors):
    rows=query(backend,'_exec_warnings',[],dict(execErrorPaths=paths,execErrors=errors))
    if any(r['code']!='EXEC_WARNING' for r in rows):raise ValueError('Unexpected executor read warning')
    for r in sorted(rows,key=lambda r:r['parameters']['ordinal']):sys.stderr.write(r['parameters']['text'])


def sources(backend,entry,values):
    files=[dict(path=f'input/{i}/source.py',source=s) for i,s in enumerate(values)]
    rows=query(backend,entry,files,dict(apiInputDirs=['input']))
    if any(r['code']!='run_in_executor' for r in rows):raise ValueError('Unexpected executor finding')
    by_file={f['path']:[] for f in files}
    for row in sorted(rows,key=lambda r:r['line']):by_file[row['path']].append((row['line'],row['parameters']['source']))
    return [by_file[f['path']] for f in files]


def scan(backend,entry,module,args):
    if entry in {'should_check_file','is_allowed'}:
        rows=query(backend,entry,[],dict(path=str(args['filepath'])))
        code='EXECUTOR_STRICT' if entry=='should_check_file' else 'EXECUTOR_ALLOWED'
        if len(rows)>1 or any(r['code']!=code for r in rows):raise ValueError('Invalid executor path predicate')
        return bool(rows)
    if entry=='check_source':content=args['source']
    else:
        try:content=Path(args['filepath']).read_text()
        except Exception as error:
            sources(backend,entry,[]);warnings(backend,[str(args['filepath'])],[str(error)]);return []
    return sources(backend,entry,[content])[0]


def install(backend,module,detectors=False):
    rows=query(backend,'_exec_configuration',[])
    if not rows or any(r['code']!='EXEC_CONFIG' for r in rows):raise ValueError('Missing executor configuration')
    config={r['parameters']['key']:r['parameters']['value'] for r in rows}
    for name,key in [('STRICT_DIRECTORIES','strict'),('ALLOWED_PATTERNS','allowed')]:backend.replace(module,name,json.loads(config[key]))
    for name,(pattern,flags) in json.loads(config['patterns']).items():backend.replace(module,name,re.compile(pattern,flags))
    data=json.loads(config['extractor'])
    for name in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:
        if name in data:data[name]=tuple(data[name])
    backend.replace(module,'ASYNC_EXECUTOR_EXTRACTOR',type(module.ASYNC_EXECUTOR_EXTRACTOR)(**data))
    if hasattr(backend,'configuration_bindings'):backend.configuration_bindings[CHECK]=dict(nativeCall=len(backend.calls)-1,names=['STRICT_DIRECTORIES','ALLOWED_PATTERNS','EXECUTOR_PATTERN','NOQA_PATTERN','ASYNC_EXECUTOR_EXTRACTOR'])
    def extract(_path,source):return [dict(line=l,source=s) for l,s in sources(backend,'_extract_async_executor_facts',[source])[0]]
    backend.replace(module,'_extract_async_executor_facts',extract)
    backend.replace(module,'async_executor_fact_extractors',lambda _root:{module.ASYNC_EXECUTOR_EXTRACTOR.extractor_id:extract})
    module._rerail_executor_extract=lambda path,source:SimpleNamespace(facts={module.ASYNC_EXECUTOR_EXTRACTOR.extractor_id:extract(path,source)},errors={})
    def plugin(_root):
        scope=json.loads(config['scope'])
        for name in ['directories','explicit_files']:scope[name]=tuple(Path(p) for p in scope[name])
        def report(view):
            family=module.ASYNC_EXECUTOR_EXTRACTOR.extractor_id
            bundles=encode_bundles([dict(path=str(p),source_path=str(p),value=b.facts.get(family,[])) for p,b in view.files.items()])
            rows=query(backend,'_report_facts',[dict(path='reporter-facts.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))])
            if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected executor reporter result')
            ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
            return [module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in ordered]
        return module.LinterPlugin(**json.loads(config['plugin']),file_scope=module.FileScopeSpec(**scope),required_facts=(module.ASYNC_EXECUTOR_EXTRACTOR,),reporter=report)
    backend.replace(module,'async_executor_plugin',plugin)
    discovery_globals=module.resolve_root_and_dirs.__globals__
    def resolve(args,*,default_dirs=None):
        rows=query(backend,'_exec_cli_root',[],dict(execExplicitRoot=str(int(args.root is not None)),execRoot=args.root or '',execDiscoverySource=str(Path(discovery_globals['__file__']).resolve()),execHasDirs=str(int(args.dirs is not None)),execDirs=list(args.dirs or []),execDefaults=list(default_dirs or [])))
        root=Path(one(rows,'EXEC_CLI_ROOT','path')).resolve();raw=[r['parameters']['path'] for r in sorted((r for r in rows if r['code']=='EXEC_CLI_ARG'),key=lambda r:r['parameters']['ordinal'])]
        rows=query(backend,'_exec_cli_dirs',[],dict(env=['executor.paths=true',*[f'data-layer.scan.{i}={p}' for i,p in enumerate(raw)]]),root=root)
        if any(r['code']!='EXEC_CLI_DIR' for r in rows):raise ValueError('Unexpected executor CLI directory')
        return root,[r['parameters']['path'] for r in sorted(rows,key=lambda r:tuple(r['parameters'][k] for k in ['argument','rank','path']))]
    backend.replace(module,'resolve_root_and_dirs',resolve)
    def cli():
        schema=json.loads(config['cli_schema']);parser=argparse.ArgumentParser(description=schema['description'])
        for item in schema['arguments']:
            kwargs=dict(item['kwargs'])
            if 'type' in kwargs:kwargs['type']={'str':str}[kwargs['type']]
            parser.add_argument(*item['names'],**kwargs)
        args=parser.parse_args();root,dirs=module.resolve_root_and_dirs(args,default_dirs=json.loads(config['cli_default_dirs']));paths=[]
        for directory in dirs:
            p=root/directory
            if p.exists():paths.extend(p.rglob('*.py'))
        rows=query(backend,'_exec_cli_select',[],dict(execCandidates=[str(p) for p in paths],execRoot=str(root),execExplicitRoot=str(int(args.root is not None)),execVerbose=str(int(args.verbose))))
        if any(r['code'] not in {'EXEC_CLI_READ','EXEC_VERBOSE'} for r in rows):raise ValueError('Unexpected executor CLI selection')
        for row in sorted((r for r in rows if r['code']=='EXEC_VERBOSE'),key=lambda r:r['parameters']['ordinal']):sys.stdout.write(row['parameters']['text'])
        selected=[r['parameters'] for r in sorted((r for r in rows if r['code']=='EXEC_CLI_READ'),key=lambda r:r['parameters']['ordinal'])]
        if getattr(module.check_file,'_rerail_native_check',None)==CHECK:
            texts=[];errors=[];error_paths=[]
            for p in selected:
                try:texts.append(Path(p['path']).read_text())
                except Exception as error:texts.append('');errors.append(str(error));error_paths.append(p['path'])
            values=sources(backend,'_exec_batch',texts)
            if errors:warnings(backend,error_paths,errors)
            if hasattr(backend,'compare_original'):
                for p,value in zip(selected,values,strict=True):backend.compare_original(CHECK,'check_file',module,(Path(p['path']),),{},value)
        else:values=[module.check_file(Path(p['path'])) for p in selected]
        files=[dict(path=p['relative'],group=p['group'],violations=v) for p,v in zip(selected,values,strict=True)]
        rows=query(backend,'_exec_cli_report',[dict(path='executor-cli.json',source=json.dumps(dict(files=files),ensure_ascii=False))],dict(execStrict=str(int(args.strict))))
        if any(r['code'] not in {'EXEC_CLI_OUTPUT','EXEC_CLI_EXIT'} for r in rows):raise ValueError('Unexpected executor CLI report')
        for row in sorted((r for r in rows if r['code']=='EXEC_CLI_OUTPUT'),key=lambda r:tuple(r['parameters'][k] for k in ['section','path_key','part','ordinal'])):sys.stdout.write(row['parameters']['text'])
        return one(rows,'EXEC_CLI_EXIT','status')
    backend.replace(module,'cli_main',cli)
    if detectors:
        for entry in ENTRIES:
            original=getattr(module,entry);signature=inspect.signature(original)
            def bind(entry,original,signature):
                @wraps(original)
                def run(*args,**kwargs):
                    bound=signature.bind(*args,**kwargs);bound.apply_defaults();return scan(backend,entry,module,bound.arguments)
                run._rerail_native_check=CHECK;return run
            backend.replace(module,entry,bind(entry,original,signature))
