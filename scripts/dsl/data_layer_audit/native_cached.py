# SPDX-License-Identifier: Apache-2.0
"""Marshal cached-policy discovery and fingerprints from shared native producers."""
import json
from pathlib import Path

from native_scanners import logical


def install(backend):
    module=backend.modules['cached_pure_lazy_allowlist']
    def records(files,contract,module_name=None,paths=None):
        parameters={} if module_name is None else {'module_name':module_name}
        rows=backend.execute('cached_pure_lazy_allowlist','discovery',files,contract=contract,parameters=parameters)
        for row in rows:
            if row['code']=='NATIVE-SYNTAX':
                error=row['variables']['syntax_error']
                path=str(paths[row['path']]) if paths is not None else row['path']
                raise SyntaxError(error['msg'],(path,error['lineno'],error['offset'],error['text'],error['end_lineno'],error['end_offset']))
        return [json.loads(row['variables']['row']['payload']) for row in rows]
    def convert(value,paths,memoized=False):
        value['path']=paths[value['path']]
        for name in ('cache_args','cache_annotations','cache_members','errors'):
            if name in value: value[name]=tuple(tuple(item) if isinstance(item,list) else item for item in value[name])
        return (module.MemoizedDecoratedFunction if memoized else module.DecoratedFunction)(**value)
    def scan(path,source,*,module_name):
        path=Path(path);backend.fact_sources[path]=source
        name=logical(path,module.REPO_ROOT)
        return tuple(convert(row,{name:path}) for row in records([dict(path=name,source=source)],'cached-discovery',module_name,{name:path}))
    backend.replace(module,'scan_cached_pure_lazy_source',scan)
    def discover(root=module.REPO_ROOT,*,memoized=False):
        root=Path(root)
        roots=[root/name for name in module.FIXED_SCAN_ROOTS]+sorted(p for p in root.glob('l[0-9][0-9][0-9][0-9]_*') if p.is_dir())
        paths=sorted({p for directory in roots if directory.exists() for p in directory.rglob('*.py') if 'tests' not in p.parts and not p.name.startswith('test_')})
        files=[];identities={}
        for path in paths:
            source=path.read_text(encoding='utf-8');backend.fact_sources[path]=source
            name=logical(path,root);files.append(dict(path=name,source=source));identities[name]=path
        rows=records(files,'memoized-discovery' if memoized else 'cached-discovery',paths=identities)
        return tuple(sorted((convert(row,identities,memoized) for row in rows),key=lambda item:item.function))
    backend.replace(module,'discover_cached_pure_lazy_functions',discover)
    backend.replace(module,'discover_memoized_pure_lazy_functions',lambda root=module.REPO_ROOT:discover(root,memoized=True))
    def runtime(path=module.PURITY_SOURCE,*,policy_path=module.POLICY_SOURCE,optimizer_cache_path=module.OPTIMIZER_CACHE_SOURCE):
        files=[dict(path=name,source=Path(source).read_text(encoding='utf-8')) for name,source in (
            ('l2055_lazy_result/purity.py',path),('l2055_lazy_result/cached_pure_lazy_policy.py',policy_path),('l2070_batching/optimizer_run_cache.py',optimizer_cache_path))]
        rows=backend.execute('cached_pure_lazy_allowlist','runtime',files,contract='runtime-fingerprint')
        if len(rows)!=1: raise AssertionError('Native runtime hash must return exactly one result')
        value=rows[0]['variables']['row'];missing=json.loads(value['missing'])
        if missing: raise RuntimeError(f'Missing cached pure-lazy runtime symbols: {tuple(missing)!r}')
        return value['hash']
    backend.replace(module,'cached_pure_lazy_runtime_source_hash',runtime)
