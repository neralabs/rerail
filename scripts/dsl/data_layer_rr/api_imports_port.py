"""API-import I/O, traversal-plan execution and passive DTO transport."""
import json
import os
from pathlib import Path

CHECK='no_api_root_imports'


def scan_batch(suite,module,root,paths,entry='check_file',options=None):
    files=[];owners={}
    for i,path in enumerate(paths):
        owner=root/path
        try:source=owner.read_text()
        except (OSError,UnicodeDecodeError):continue
        key=f'file-{i}/{owner.name}';owners[key]=(i,owner);files.append(dict(path=key,source=source))
    metadata=dict(nariBatch='true',nariRoot=str(root),ownerDocs=[f['path'] for f in files],
        ownerPaths=[str(owners[f['path']][1]) for f in files],**(options or {}))
    rows=suite.native(CHECK,entry,files,metadata)
    if any(r['code']!='NARI001' for r in rows):raise AssertionError('Unexpected native API-import diagnostic')
    rows.sort(key=lambda r:(owners[r['path']][0],r['parameters']['ordinal'],r['parameters'].get('piece',0),r['parameters']['pattern'],r['parameters']['occurrence']))
    return [module.Violation(owners[r['path']][1],r['line'],r['message']) for r in rows]


def scan(suite,entry,module,arguments):
    path=Path(arguments['file_path']);source=arguments.get('source')
    if entry=='check_file':
        try:source=path.read_text()
        except (OSError,UnicodeDecodeError):return []
    return scan_batch(suite,module,module.get_project_root(),[path],entry,dict(providedSource=source))


def install(suite,module,detectors=False):
    import re
    import sys
    rows=suite.native(CHECK,'_ni_configuration',[])
    if not rows or any(r['code']!='NI_CONFIG' for r in rows):raise AssertionError('Missing native API-import configuration')
    def values(key):return [p['value'] for p in sorted((r['parameters'] for r in rows if r['parameters']['key']==key),key=lambda p:p['ordinal'])]
    for i,name in enumerate(['ROOT_IMPORT_RE','ROOT_IMPORT_TEXT_RE']):suite.replace(module,name,re.compile(values('regex')[i],int(values('flags')[i])))
    data=json.loads(values('extractor')[0])
    for key in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:data[key]=tuple(data[key])
    suite.replace(module,'NO_API_ROOT_IMPORTS_EXTRACTOR',type(module.NO_API_ROOT_IMPORTS_EXTRACTOR)(**data))
    if hasattr(suite,'configuration_bindings'):suite.configuration_bindings[CHECK]=dict(nativeCall=len(suite.calls)-1,names=['ROOT_IMPORT_RE','ROOT_IMPORT_TEXT_RE','NO_API_ROOT_IMPORTS_EXTRACTOR'])
    def python_files(root,exclude_dirs):
        plan=suite.native(CHECK,'_ni_walk_plan',[],dict(excludeDirs=list(exclude_dirs)))
        if any(r['code']!='NI_PRUNE' for r in plan):raise AssertionError('Invalid native traversal plan')
        pruned={r['parameters']['value'] for r in plan};files=[]
        # Execute the native traversal plan, then enforce all file predicates in SQL.
        for directory,names,filenames in os.walk(root):
            names[:]=[name for name in names if name not in pruned]
            files.extend((Path(directory)/name).relative_to(root).as_posix() for name in filenames)
        response=suite.native(CHECK,'_ni_walk_files',[],dict(walkFiles=files,excludeDirs=list(exclude_dirs)))
        if any(r['code']!='NI_FILE' for r in response):raise AssertionError('Invalid native discovery result')
        return [root/r['parameters']['value'] for r in sorted(response,key=lambda r:r['parameters']['ordinal'])]
    def convert(entry,payload):
        response=suite.native(CHECK,entry,[dict(path='api-imports-values.json',source=json.dumps(dict(rows=payload),ensure_ascii=False))])
        if any(r['code']!='NI_VALUE' for r in response):raise AssertionError('Invalid native fact projection')
        response.sort(key=lambda r:r['parameters']['ordinal'])
        if [r['parameters']['ordinal'] for r in response]!=list(range(len(payload))):raise AssertionError('Native projection lost rows')
        return [json.loads(r['parameters']['value_json']) for r in response]
    def from_fact(raw_fact,fallback_path):
        value=convert('_ni_from_fact',[dict(raw=raw_fact,fallback=str(fallback_path))])[0]
        return None if value is None else module.Violation(Path(value['file_path']),value['line'],value['message'])
    def to_fact(violation):
        return convert('_ni_to_fact',[dict(raw=dict(file_path=str(violation.file_path),line=violation.line_number,message=violation.message))])[0]
    def cli_main():
        root=module.get_project_root();files=module.find_python_files(root,set(values('exclude')))
        violations=module._violations_for_files(root,tuple(files))
        response=suite.native(CHECK,'_ni_cli_report',[],dict(sourceRoot=str(root),argv=list(sys.argv),
            cliPath=[str(v.file_path) for v in violations],cliLine=[str(v.line_number) for v in violations],cliMessage=[v.message for v in violations]))
        if any(r['code'] not in {'NI_OUTPUT','NI_EXIT','NI_PATH_ERROR'} for r in response):raise AssertionError('Invalid native CLI output')
        for p in sorted((r['parameters'] for r in response if r['code']=='NI_OUTPUT'),key=lambda p:(p['phase'],p['path_key'],p['line'],p['ordinal'])):sys.stdout.write(p['stdout'])
        bad=[r['parameters'] for r in response if r['code']=='NI_PATH_ERROR']
        if bad:
            p=min(bad,key=lambda p:(p['path'],p['line'],p['ordinal']));Path(p['path']).relative_to(root)
            raise AssertionError('Native path error unexpectedly succeeded')
        status=[r['parameters']['value'] for r in response if r['code']=='NI_EXIT']
        if len(status)!=1:raise AssertionError('Missing native CLI status')
        return status[0]
    suite.replace(module,'get_project_root',lambda:Path(module.__file__).parent.parent)
    suite.replace(module,'find_python_files',python_files)
    suite.replace(module,'_violations_for_files',lambda root,files:scan_batch(suite,module,root,files))
    suite.replace(module,'_violation_from_fact',from_fact);suite.replace(module,'_violation_to_fact',to_fact)
    suite.replace(module,'cli_main',cli_main)
    if detectors:
        suite.replace(module,'check_file',lambda file_path:scan(suite,'check_file',module,dict(file_path=file_path)))
        suite.replace(module,'check_file_content',lambda file_path,source,tree=None:scan(suite,'check_file_content',module,dict(file_path=file_path,source=source,tree=tree)))
