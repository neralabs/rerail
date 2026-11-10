"""Transport dynamic-loading text and passive tuples; native rules own detection."""
import operator
import json
import re
from pathlib import Path

CHECK='dynamic_module_loading'


def scan(backend,entry,module,path,root,allowed,content=None):
    path=Path(path);root=Path(root);logical=path.relative_to(root).as_posix()
    allowed=list(allowed)
    if entry=='_scan_file':
        plan=backend.native(CHECK,'_dynamic_prepare',[],dict(dmlCandidates=[logical],allowlistedFiles=allowed))
        if any(r['code']!='DML_READ' or r['parameters']['ordinal']!=0 for r in plan):raise ValueError('Unexpected native dynamic read plan')
        if not plan:return []
        try:content=path.read_text(encoding='utf-8')
        except (OSError,UnicodeDecodeError):content=None
    files=[] if content is None else [dict(path='source-0.py',source=content)]
    rows=backend.native(CHECK,entry,files,dict(allowlistedFiles=list(allowed),dmlDocuments=['source-0.py'],dmlPaths=[logical]))
    if any(r['code']!='DML001' for r in rows):raise ValueError('Unexpected native dynamic-loading row')
    return [(path,r['line'],r['parameters']['snippet'],r['parameters']['label']) for r in sorted(rows,key=lambda r:(r['line'],r['parameters']['ordinal']))]


def configuration(backend):
    rows=backend.native(CHECK,'_dynamic_configuration',[])
    if any(r['code'] not in {'DML_CONFIG','DML_CONFIG_KEY'} for r in rows):raise ValueError('Unexpected dynamic-loading configuration')
    grouped={r['parameters']['key']:[] for r in rows if r['code']=='DML_CONFIG_KEY'}
    for row in rows:
        if row['code']=='DML_CONFIG':
            p=row['parameters'];grouped[p['key']].append((p['ordinal'],p['value']))
    return {key:[v for _,v in sorted(grouped[key])] for key in sorted(grouped)}


def repository(backend,module,root,directories=None):
    root=Path(root)
    exports=['dml.discovery=true']+[f'dml.allowed.{i}={p}' for i,p in enumerate(module.ALLOWLISTED_FILES)]
    if directories is not None:exports+=['data-layer.scan.configured=true']+[f'data-layer.scan.{i}={p}' for i,p in enumerate(directories)]
    rows=backend.native(CHECK,'_dynamic_repository',[],dict(env=exports),root=root)
    if any(r['code'] not in {'DML_DISCOVERED','DML_FOUND'} for r in rows):raise ValueError('Unexpected dynamic repository response')
    found=sorted((r for r in rows if r['code']=='DML_FOUND'),key=lambda r:(r['parameters']['file_ordinal'],r['line'],r['parameters']['ordinal']))
    return [(root/r['parameters']['path'],r['line'],r['parameters']['snippet'],r['parameters']['label']) for r in found]


def install(backend,module,detectors=False):
    config=configuration(backend)
    if not config:raise ValueError('Native dynamic-loading configuration is missing')
    backend.replace(module,'LINTER_NAME',config['linter'][0]);backend.replace(module,'ALLOWLIST_SIGNATURE',config['signature'][0])
    backend.replace(module,'ALLOWLISTED_FILES',list(config['allowed']));backend.replace(module,'_FAST_SUBSTRINGS',tuple(config['fast']))
    for prefix,name in [('primary','_PATTERNS'),('imports','_IMPORT_MODULE_PATTERNS')]:
        values=list(zip(config[prefix+'.pattern'],config[prefix+'.flags'],config[prefix+'.label'],strict=True))
        backend.replace(module,name,[(re.compile(p,int(f)),label) for p,f,label in values])
    data=json.loads(config['extractor'][0])
    for key in ['source_triggers','source_trigger_groups','project_input_facts']:data[key]=tuple(data[key])
    backend.replace(module,'DYNAMIC_MODULE_LOADING_EXTRACTOR',type(module.DYNAMIC_MODULE_LOADING_EXTRACTOR)(**data))
    if detectors:
        backend.replace(module,'find_violations',lambda root,directories=None:repository(backend,module,root,directories))
        backend.replace(module,'_scan_file',lambda file_path,root,allowlisted:scan(backend,'_scan_file',module,file_path,root,allowlisted))
        backend.replace(module,'_scan_file_content',lambda file_path,root,allowlisted,content:scan(backend,'_scan_file_content',module,file_path,root,allowlisted,content))
        def predicate(lines,line_idx):
            lines[line_idx]  # Passive list-index validation, before any native query.
            rows=backend.native(CHECK,'_is_in_type_checking_block',[],dict(helperLines=lines,lineIndex=operator.index(line_idx)))
            if len(rows)>1 or any(r['code']!='DML_TYPE_CHECKING' for r in rows):raise ValueError('Unexpected TYPE_CHECKING result')
            return bool(rows)
        backend.replace(module,'_is_in_type_checking_block',predicate)
    module._rerail_dynamic_configuration=lambda:configuration(backend)
