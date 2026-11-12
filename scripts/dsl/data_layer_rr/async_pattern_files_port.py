"""Collect filesystem inputs in batches and reconstruct native async-pattern DTOs."""
import os
from pathlib import Path
import sys

CHECK='async_patterns'
CODES={'ASYNC001','ASYNC002','ASYNC004','ASYNC005','ASYNC006','ASYNC007','ASYNC008'}


def query(backend,entry,files=(),options=None):return backend.native(CHECK,entry,list(files),options or {})


def select(backend,paths,relative=None):
    options=dict(apCandidatePaths=[str(p) for p in paths],apCandidateIsFile=[str(int(p.is_file())) for p in paths])
    if relative is not None:options['apCandidateRelative']=relative
    rows=query(backend,'_ap_file_select' if relative is None else '_ap_directory_select',options=options)
    if any(r['code']!='AP_FILE' for r in rows):raise ValueError('Unexpected async-pattern selected path')
    return [Path(r['parameters']['path']) for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]


def batch(backend,module,paths,*,directory=False):
    files=[];owners={};errors=[];error_paths=[]
    for i,path in enumerate(paths):
        try:source=path.read_text(encoding='utf-8')
        except (SyntaxError,UnicodeDecodeError) as error:
            source=''
            if directory:error_paths.append(str(path));errors.append(str(error))
        except OSError:
            if directory:raise
            source=''
        key=f'api/_ap_buffers/{i}.py';files.append(dict(path=key,source=source));owners[key]=(i,path)
    rows=query(backend,'_ap_file_batch',files,dict(apBatchKeys=list(owners),apBatchPaths=[str(p) for _,p in owners.values()]))
    counts=[r['parameters'] for r in rows if r['code']=='AP_COUNTS']
    if len(counts)!=1 or any(r['code'] not in CODES|{'AP_COUNTS'} for r in rows):raise ValueError('Unexpected async-pattern file batch')
    ordered=sorted((r for r in rows if r['code'] in CODES),key=lambda r:(owners[r['path']][0],r['parameters']['ordinal'],r['parameters']['rank']))
    result=[module.Violation(owners[r['path']][1],r['line'],r['code'],r['message'],r['parameters']['context'],bool(r['parameters']['fallback'])) for r in ordered]
    if errors:
        warnings=query(backend,'_ap_read_warnings',options=dict(apErrorPaths=error_paths,apErrors=errors))
        if any(r['code']!='AP_WARNING' for r in warnings):raise ValueError('Unexpected async-pattern warning')
        for row in sorted(warnings,key=lambda r:r['parameters']['ordinal']):sys.stderr.write(row['parameters']['text'])
    return result,counts[0]


def directories(backend,module,root,names):
    paths=[];relative=[]
    for name in names:
        base=root/name
        if base.exists():
            for directory,_dirs,filenames in os.walk(base):
                for filename in filenames:
                    p=Path(directory)/filename;paths.append(p);relative.append(p.relative_to(base).as_posix())
    selected=select(backend,paths,relative);violations,counts=batch(backend,module,selected,directory=True)
    return module.LintResults(violations,counts['files_checked'],counts['files_with_violations'])


def directory(backend,module,root,name):return directories(backend,module,root,[name])


def api(backend,module,root,paths):
    rows=query(backend,'_ap_api_transactions',options=dict(apApiPresent=[str(int((root/p).exists())) for p in paths]))
    if len(rows)!=1 or rows[0]['code']!='AP_API_STATUS':raise ValueError('One native API abstraction status required')
    p=rows[0]['parameters']
    if p['error_type']:raise {'RuntimeError':RuntimeError}[p['error_type']](p['detail'])
    return module.LintResults(files_checked=p['files_checked'])


def install(backend,module,config):
    backend.replace(module,'_violations_for_files',lambda root,files:batch(backend,module,select(backend,list(files)))[0])
    backend.replace(module,'lint_directory',lambda base_path,rel_path:directory(backend,module,base_path,rel_path))
    backend.replace(module,'lint_api_transactions',lambda base_path:api(backend,module,base_path,config['api_transaction_files']))
    backend.replace(module.LintResults,'add',lambda owner,value:owner.violations.append(value))
