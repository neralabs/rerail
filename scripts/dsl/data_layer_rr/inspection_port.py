"""Transport inspection source batches; native rules own selection and facts."""
import json
from pathlib import Path

CHECK='no_source_inspection'


def scan(suite,entry,module,arguments):
    files=[];owners={}
    if entry=='scan_file_content':
        owners['input-0.py']=Path(arguments['path'])
        files=[dict(path='input-0.py',source=arguments['source'])]
    else:
        root=Path(arguments['root']);directories=arguments.get('directories')
        directories=module.TEST_DIRECTORIES if directories is None else directories
        candidates=[item for directory in directories for item in (root/directory).rglob('*')]
        selected=suite.native(CHECK,'_inspection_prepare',[],dict(inspectionCandidates=[str(p) for p in candidates]))
        if any(r['code']!='NSI_READ' for r in selected):raise AssertionError('Unexpected inspection read plan')
        for r in sorted(selected,key=lambda r:r['parameters']['ordinal']):
            i=r['parameters']['ordinal'];path=candidates[i]
            try:source=path.read_text(encoding='utf-8')
            except (OSError,UnicodeDecodeError):continue
            key=f'input-{i}.py';owners[key]=path;files.append(dict(path=key,source=source))
    metadata=dict(inspectionDocs=[f['path'] for f in files],inspectionPaths=[str(owners[f['path']]) for f in files])
    rows=suite.native(CHECK,entry,files,metadata)
    allowed={'NSI001','NSI_FACT'} if entry=='scan_file_content' else {'NSI001'}
    if any(r['code'] not in allowed for r in rows):raise AssertionError('Unexpected inspection finding')
    order={f['path']:i for i,f in enumerate(files)}
    rows.sort(key=lambda r:(order[r['path']],r['line'],r['parameters']['ordinal']))
    if entry=='scan_file_content':return [json.loads(r['parameters']['value_json']) for r in rows if r['code']=='NSI_FACT']
    return [(owners[r['path']],r['line'],r['parameters']['snippet'],r['parameters']['label']) for r in rows]


def install(suite,module,detectors=False):
    rows=suite.native(CHECK,'_inspection_configuration',[])
    if len(rows)!=1 or rows[0]['code']!='NSI_CONFIG':raise AssertionError('Missing native extractor configuration')
    data=json.loads(rows[0]['parameters']['value'])
    for key in ['source_triggers','source_trigger_groups','project_input_facts']:data[key]=tuple(data[key])
    suite.replace(module,'NO_SOURCE_INSPECTION_EXTRACTOR',type(module.NO_SOURCE_INSPECTION_EXTRACTOR)(**data))
    if hasattr(suite,'configuration_bindings'):suite.configuration_bindings[CHECK]=dict(nativeCall=len(suite.calls)-1,names=['NO_SOURCE_INSPECTION_EXTRACTOR'])
    if detectors:
        suite.replace(module,'scan_file_content',lambda path,source:scan(suite,'scan_file_content',module,dict(path=path,source=source)))
        suite.replace(module,'find_violations',lambda root,directories=None:scan(suite,'find_violations',module,dict(root=root,directories=directories)))
    from text_cli_port import install as cli_install
    cli_install(suite,module,CHECK)
