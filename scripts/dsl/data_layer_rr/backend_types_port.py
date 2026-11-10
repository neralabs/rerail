"""Transport backend scanner inputs and DTOs; native rules select paths and findings."""
from pathlib import Path
import json


def scan(suite, entry, module, arguments):
    if entry == 'should_skip_file':
        rows = suite.native('backend_type_strings', entry, [], dict(path=str(arguments['path'])))
        if len(rows) > 1 or any(row['code'] != 'BTS_SKIP' for row in rows):
            raise AssertionError('Invalid native backend skip predicate')
        return bool(rows)
    if entry == 'find_python_files':
        candidates,kinds = [],[]
        for raw in arguments['paths']:
            path = Path(raw)
            if path.is_file():
                candidates.append(str(path))
                kinds.append('direct')
            elif path.is_dir():
                # Transport the directory inventory. Native suffix selection and set semantics decide the result.
                items=list(path.rglob('*'));candidates.extend(str(item) for item in items);kinds.extend('walk' for _ in items)
        rows = suite.native('backend_type_strings', entry, [], dict(candidates=candidates,candidateKinds=kinds))
        if any(row['code'] != 'BTS_FILE' for row in rows):
            raise AssertionError('Invalid native backend file inventory')
        return [Path(row['parameters']['result']) for row in sorted(rows,key=lambda r:r['parameters']['order_key'])]
    path = Path(arguments['path'])
    options = dict(sourcePath=str(path))
    predicate = module.should_skip_file
    if not getattr(predicate,'_rerail_native_skip',False) and getattr(predicate, '__module__', None) != module.__name__:
        # A caller-supplied predicate is an API input. The original source predicate never executes here.
        options['skipOverride'] = str(bool(predicate(path))).lower()
    files = []
    try:
        content = arguments['content'] if entry == 'check_file_content' else path.read_text()
    except Exception:
        pass  # The source file API catches all read failures and returns no findings.
    else:
        files.append(dict(path='input.py', source=content))
    rows = suite.native('backend_type_strings', entry, files, options)
    if any(row['code'] not in {'BT000', 'BT001', 'BT002', 'BT003', 'BT004', 'BT005', 'BTS_SKIPPED'} for row in rows):
        raise AssertionError('Unexpected native backend type diagnostic')
    rows = [row for row in rows if row['code'] != 'BTS_SKIPPED']
    rows.sort(key=lambda row: (row['line'], row['parameters']['pattern'], row['parameters']['occurrence']))
    return [module.Violation(file=path, line=row['line'], column=row['parameters']['column'],
        message=row['message'], code=row['code'], snippet=row['parameters']['snippet']) for row in rows]


def scan_batch(suite,module,paths):
    paths=list(paths);predicate=module.should_skip_file
    custom=not getattr(predicate,'_rerail_native_skip',False) and getattr(predicate,'__module__',None)!=module.__name__
    overrides=[str(bool(predicate(p))).lower() if custom else 'default' for p in paths]
    metadata=dict(batchPaths=[str(p) for p in paths],batchSkipOverrides=overrides)
    prepared=suite.native('backend_type_strings','_bts_prepare',[],metadata)
    if any(r['code'] not in {'BTS_READ','BTS_SKIPPED'} for r in prepared):raise AssertionError('Invalid native read plan')
    groups=[dict(path=p,values=[],diagnostics=[]) for p in paths]
    files=[];indices=[];owners={}
    for r in prepared:
        if r['code']=='BTS_SKIPPED':groups[r['parameters']['ordinal']]['diagnostics'].append(r)
    for r in sorted((r for r in prepared if r['code']=='BTS_READ'),key=lambda r:r['parameters']['ordinal']):
        i=r['parameters']['ordinal'];path=paths[i]
        try:content=path.read_text()
        except Exception:continue
        if custom:overrides[i]=str(bool(predicate(path))).lower()
        key=f'batch-{i}/{path.name}';owners[key]=i;indices.append(str(i));files.append(dict(path=key,source=content))
    rows=suite.native('backend_type_strings','check_file',files,dict(metadata,btsBatch='true',batchDocs=[f['path'] for f in files],batchDocIndices=indices))
    for r in rows:
        if r['code']=='BTS_SKIPPED':continue
        if r['code'] not in {'BT000','BT001','BT002','BT003','BT004','BT005'}:raise AssertionError('Invalid batch diagnostic')
        groups[owners[r['path']]]['diagnostics'].append(r)
    values=[]
    for group in groups:
        diagnostics=[r for r in group['diagnostics'] if r['code']!='BTS_SKIPPED']
        diagnostics.sort(key=lambda r:(r['line'],r['parameters']['pattern'],r['parameters']['occurrence']))
        group['values']=[module.Violation(group['path'],r['line'],r['parameters']['column'],r['message'],r['code'],r['parameters']['snippet']) for r in diagnostics]
        values.extend(group['values'])
    return values,groups


def check_files(suite,module,paths):
    values,groups=scan_batch(suite,module,paths)
    if hasattr(suite,'compare_original'):
        for group in groups:suite.compare_original('backend_type_strings','check_file',module,(group['path'],),{},group['values'],native_diagnostics=group['diagnostics'])
    return values


def install(suite, module,detectors=False):
    """Back unchanged constant assertions with one native configuration batch."""
    import re
    rows = suite.native('backend_type_strings', 'configuration', [])
    if not rows or any(row['code'] != 'BTS_CONFIG' for row in rows):
        raise AssertionError('Missing native backend type configuration')
    tables = {}
    for row in sorted(rows, key=lambda row: (row['parameters']['key'], row['parameters']['ordinal'])):
        values = row['parameters']
        tables.setdefault(values['key'], []).append(values['value'])
    suite.replace(module, 'BACKEND_STRINGS', frozenset(tables['BACKEND_STRINGS']))
    suite.replace(module, 'SKIP_PATTERNS', tables['SKIP_PATTERNS'])
    suite.replace(module, 'COMPARISON_PATTERNS', [re.compile(value) for value in tables['COMPARISON_PATTERNS']])
    if not hasattr(suite, 'configuration_bindings'):
        suite.configuration_bindings = {}
    suite.configuration_bindings['backend_type_strings'] = dict(nativeCall=len(suite.calls) - 1,
        names=['BACKEND_STRINGS', 'SKIP_PATTERNS', 'COMPARISON_PATTERNS'])
    data=json.loads(tables['EXTRACTOR'][0])
    for key in ['source_triggers','source_trigger_groups','project_input_facts']:data[key]=tuple(data[key])
    suite.replace(module,'BACKEND_TYPE_STRINGS_EXTRACTOR',type(module.BACKEND_TYPE_STRINGS_EXTRACTOR)(**data))
    def fact(violation):
        payload=dict(file=str(violation.file),line=violation.line,column=violation.column,message=violation.message,code=violation.code,snippet=violation.snippet)
        response=suite.native('backend_type_strings','_bts_fact',[dict(path='backend-values.json',source=json.dumps(dict(values=[payload]),ensure_ascii=False))])
        if len(response)!=1 or response[0]['code']!='BTS_FACT':raise AssertionError('Missing native fact projection')
        return json.loads(response[0]['parameters']['value_json'])
    suite.replace(module,'_violation_to_fact',fact)
    if detectors:
        for entry in ['should_skip_file','find_python_files','check_file','check_file_content']:
            def callback(entry):
                if entry=='should_skip_file':
                    def run(path):return scan(suite,entry,module,dict(path=path))
                    run._rerail_native_skip=True
                elif entry=='find_python_files':
                    def run(paths):return scan(suite,entry,module,dict(paths=paths))
                elif entry=='check_file':
                    def run(path):return scan(suite,entry,module,dict(path=path))
                    run._rerail_native_check='backend_type_strings'
                else:
                    def run(path,content):return scan(suite,entry,module,dict(path=path,content=content))
                return run
            suite.replace(module,entry,callback(entry))
