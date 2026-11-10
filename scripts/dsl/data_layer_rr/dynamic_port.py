"""Transport dynamic-loading text and passive tuples; native rules own detection."""
import operator
from pathlib import Path

CHECK='dynamic_module_loading'


def scan(backend,entry,module,path,root,allowed,content=None):
    path=Path(path);root=Path(root);logical=path.relative_to(root).as_posix()
    if entry=='_scan_file':
        try:content=path.read_text(encoding='utf-8')
        except (OSError,UnicodeDecodeError):content=None
    files=[] if content is None else [dict(path='source-0.py',source=content)]
    rows=backend.native(CHECK,entry,files,dict(allowlistedFiles=list(allowed),dmlDocuments=['source-0.py'],dmlPaths=[logical]))
    if any(r['code']!='DML001' for r in rows):raise ValueError('Unexpected native dynamic-loading row')
    return [(path,r['line'],r['parameters']['snippet'],r['parameters']['label']) for r in sorted(rows,key=lambda r:(r['line'],r['parameters']['ordinal']))]


def configuration(backend):
    rows=backend.native(CHECK,'_dynamic_configuration',[])
    if any(r['code']!='DML_CONFIG' for r in rows):raise ValueError('Unexpected dynamic-loading configuration')
    grouped={}
    for row in rows:
        p=row['parameters'];grouped.setdefault(p['key'],[]).append((p['ordinal'],p['value']))
    return {key:[v for _,v in sorted(values)] for key,values in grouped.items()}


def install(backend,module,detectors=False):
    if detectors:
        backend.replace(module,'_scan_file',lambda file_path,root,allowlisted:scan(backend,'_scan_file',module,file_path,root,allowlisted))
        backend.replace(module,'_scan_file_content',lambda file_path,root,allowlisted,content:scan(backend,'_scan_file_content',module,file_path,root,allowlisted,content))
        def predicate(lines,line_idx):
            lines[line_idx]  # Passive list-index validation, before any native query.
            rows=backend.native(CHECK,'_is_in_type_checking_block',[],dict(helperLines=lines,lineIndex=operator.index(line_idx)))
            if len(rows)>1 or any(r['code']!='DML_TYPE_CHECKING' for r in rows):raise ValueError('Unexpected TYPE_CHECKING result')
            return bool(rows)
        backend.replace(module,'_is_in_type_checking_block',predicate)
    module._rerail_dynamic_configuration=lambda:configuration(backend)
