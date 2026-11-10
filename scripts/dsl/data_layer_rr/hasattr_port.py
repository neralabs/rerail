"""Transport async capability sources and file batches; native rules own selection."""
from pathlib import Path
import json

CHECK='hasattr_async'


def values(rows):
    if any(r['code'] not in {'hasattr','getattr'} for r in rows):raise AssertionError('Unexpected async scan response')
    return [(r['line'],r['code'],r['parameters']['method'])
            for r in sorted(rows,key=lambda r:(r['line'],0 if r['code']=='hasattr' else 1))]


def scan(suite,entry,arguments):
    if entry in {'is_allowed_line','should_exclude'}:
        options=dict(line=arguments['line']) if entry=='is_allowed_line' else dict(path=str(arguments['path']))
        rows=suite.native(CHECK,entry,[],options)
        code='ASYNC_ALLOWED' if entry=='is_allowed_line' else 'ASYNC_EXCLUDED'
        if len(rows)>1 or any(r['code']!=code for r in rows):raise AssertionError('Invalid async helper response')
        return bool(rows)
    files=[]
    try:source=arguments['source'] if entry=='check_source' else Path(arguments['filepath']).read_text()
    except Exception:pass
    else:files=[dict(path='input.py',source=source)]
    return values(suite.native(CHECK,entry,files))


def check_files(suite,module,paths):
    paths=list(paths)
    prepared=suite.native(CHECK,'_async_prepare',[],dict(asyncPaths=[str(p) for p in paths]))
    if any(r['code']!='ASYNC_READ' for r in prepared):raise AssertionError('Invalid async read plan')
    selected=sorted((r['parameters']['ordinal'] for r in prepared))
    if len(selected)!=len(set(selected)) or any(i<0 or i>=len(paths) for i in selected):raise AssertionError('Invalid async occurrence')
    files=[];owners={};groups={i:[] for i in selected}
    for i in selected:
        try:source=paths[i].read_text()
        except Exception:continue
        key=f'batch-{i}/input.py';owners[key]=i;files.append(dict(path=key,source=source))
    rows=suite.native(CHECK,'check_file',files)
    for row in rows:groups[owners[row['path']]].append(row)
    result=[]
    for i in selected:
        findings=values(groups[i]);result.extend((paths[i],*v) for v in findings)
        if hasattr(suite,'compare_original'):suite.compare_original(CHECK,'check_file',module,(paths[i],),{},findings,native_diagnostics=groups[i])
    return result


def install(suite,module):
    rows=suite.native(CHECK,'_async_configuration',[])
    if len(rows)!=1 or rows[0]['code']!='ASYNC_CONFIG':raise AssertionError('Missing native async extractor configuration')
    data=json.loads(rows[0]['parameters']['value'])
    for key in ['source_triggers','project_input_facts']:data[key]=tuple(data[key])
    data['source_trigger_groups']=tuple(tuple(group) for group in data['source_trigger_groups'])
    suite.replace(module,'HASATTR_ASYNC_EXTRACTOR',type(module.HASATTR_ASYNC_EXTRACTOR)(**data))
    if hasattr(suite,'configuration_bindings'):
        suite.configuration_bindings[CHECK]=dict(nativeCall=len(suite.calls)-1,names=['HASATTR_ASYNC_EXTRACTOR'])
    def facts(_path,source):
        rows=suite.native(CHECK,'_async_facts',[dict(path='input.py',source=source)])
        if any(r['code'] not in {'ASYNC_FACT','hasattr','getattr'} for r in rows):raise AssertionError('Unexpected async fact response')
        return [json.loads(r['parameters']['value_json']) for r in sorted(
            (r for r in rows if r['code']=='ASYNC_FACT'),key=lambda r:(r['parameters']['line'],r['parameters']['kind_order']))]
    suite.replace(module,'_extract_hasattr_async_facts',facts)
