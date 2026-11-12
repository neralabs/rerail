"""Transport complete admin file batches; native rules select and inspect their contents."""
import inspect
from pathlib import Path

CHECK='admin_async_purity'


def scan(backend,entry,module,args):
    options=dict(allowlistedFiles=list(module.ALLOWLISTED_FILES))
    if entry in {'_should_skip','_is_allowlisted'}:
        options['path']=args.get('path',args.get('file'))
        rows=backend.native(CHECK,entry,[],options)
        code='ADMIN_SKIP' if entry=='_should_skip' else 'ADMIN_ALLOWLIST'
        if len(rows)>1 or any(r['code']!=code for r in rows):raise ValueError('Invalid native admin predicate')
        return bool(rows)
    root=Path(args.get('root',args.get('project_root')))
    if entry=='_collect_violations_from_source':
        extract=module.admin_async_purity_fact_extractors(root)[module.ADMIN_ASYNC_PURITY_EXTRACTOR.extractor_id]
        return [module.Violation(**v) for v in extract(args['path'],args['content'])['violations']]
    candidates=[]
    for target in module.SCAN_DIRS:
        path=root/target
        if path.is_file():candidates.append(path)
        elif path.is_dir():candidates.extend(sorted(path.rglob('*.py')))
    paths=[str(p.relative_to(root)) for p in candidates]
    rows=backend.native(CHECK,'_admin_select',[],dict(options,adminCandidatePaths=paths))
    if any(r['code']!='ADMIN_FILE' for r in rows):raise ValueError('Invalid native admin selection')
    if entry=='_collect_admin_async_files':
        return tuple(candidates[r['parameters']['ordinal']] for r in sorted(rows,key=lambda r:(r['parameters']['path_order'],r['parameters']['ordinal'])))
    files=[];owners=[]
    for row in sorted(rows,key=lambda r:r['parameters']['ordinal']):
        p=row['parameters'];path=candidates[p['ordinal']]
        if p['path']!=paths[p['ordinal']]:raise ValueError('Native admin selected a different file')
        try:source=path.read_text()
        except OSError:continue
        files.append(dict(path=f'l8000_admin/tools/_admin-buffers/{len(files)}.py',source=source));owners.append(p['path'])
    rows=backend.native(CHECK,'_admin_scan',files,dict(options,adminDocuments=[f['path'] for f in files],adminPaths=owners))
    if any(r['code']!='AAP001' for r in rows):raise ValueError('Invalid native admin detection output')
    return [module.Violation(file=r['path'],line=r['line'],pattern=r['parameters']['pattern'],text=r['parameters']['text'])
        for r in sorted(rows,key=lambda r:(r['parameters']['file_ordinal'],r['line'],r['parameters']['ordinal']))]


def install(backend,module):
    for entry in ['_collect_violations','_collect_violations_from_source','_collect_admin_async_files','_should_skip','_is_allowlisted']:
        signature=inspect.signature(getattr(module,entry))
        def bind(entry,signature):
            def run(*args,**kwargs):
                bound=signature.bind(*args,**kwargs);bound.apply_defaults();return scan(backend,entry,module,bound.arguments)
            return run
        backend.replace(module,entry,bind(entry,signature))
