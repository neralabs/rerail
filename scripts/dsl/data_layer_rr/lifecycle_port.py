"""Lifecycle I/O and passive DTO transport; native rules own lint and report decisions."""
import json
from pathlib import Path

CHECK='versioned_lifecycle_legacy_surfaces'


def scan_batch(suite,module,root,paths,entry='scan_file',options=None):
    files=[];owners={}
    for i,path in enumerate(paths):
        owner=root/path
        try:source=owner.read_text()
        except (OSError,UnicodeDecodeError):continue
        key=f'file-{i}.py';owners[key]=(i,owner);files.append(dict(path=key,source=source))
    metadata=dict(lifecycleBatch='true',lcRoot=str(module.ROOT),lcScopeRoot=str(root),
        ownerDocs=[f['path'] for f in files],ownerPaths=[str(owners[f['path']][1]) for f in files],**(options or {}))
    rows=suite.native(CHECK,entry,files,metadata)
    rows.sort(key=lambda r:(owners[r['path']][0],*[r['parameters'][k] for k in ['collector_order0','collector_order1','collector_order2']]))
    return [module.Violation(owners[r['path']][1],r['parameters']['cls'],r['parameters']['method'],r['line']) for r in rows]


def scan_file(suite,module,entry,path,source=None):
    if entry=='scan_file':source=path.read_text()
    return scan_batch(suite,module,module.ROOT,[path],entry,dict(lcSuppliedSource=source))


def install(suite,module,detectors=False):
    import argparse
    import sys
    rows=suite.native(CHECK,'_lc_configuration',[])
    if not rows or any(r['code']!='LC_CONFIG' for r in rows):raise AssertionError('Missing native lifecycle configuration')
    def values(key):return [p['value'] for p in sorted((r['parameters'] for r in rows if r['parameters']['key']==key),key=lambda p:p['ordinal'])]
    suite.replace(module,'BANNED_METHODS',frozenset(values('methods')))
    suite.replace(module,'DEFAULT_SCAN_ROOTS',tuple(module.ROOT/p for p in values('root_suffix')))
    data=json.loads(values('extractor')[0])
    for key in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:data[key]=tuple(data[key])
    suite.replace(module,'VERSIONED_LIFECYCLE_LEGACY_SURFACES_EXTRACTOR',type(module.VERSIONED_LIFECYCLE_LEGACY_SURFACES_EXTRACTOR)(**data))
    if hasattr(suite,'configuration_bindings'):suite.configuration_bindings[CHECK]=dict(nativeCall=len(suite.calls)-1,
        names=['BANNED_METHODS','DEFAULT_SCAN_ROOTS','VERSIONED_LIFECYCLE_LEGACY_SURFACES_EXTRACTOR'])
    def python_files(scan_roots):
        candidates=[];groups=[];kinds=[]
        for group,root in enumerate(scan_roots):
            if root.is_file():items=[root];kind='direct'
            else:items=list(root.rglob('*'));kind='walk'
            candidates.extend(str(p) for p in items);groups.extend(str(group) for _ in items);kinds.extend(kind for _ in items)
        response=suite.native(CHECK,'_lc_files',[],dict(candidatePaths=candidates,candidateGroups=groups,candidateKinds=kinds))
        if any(r['code']!='LC_FILE' for r in response):raise AssertionError('Unexpected native discovery row')
        return [Path(p['value']) for p in sorted((r['parameters'] for r in response),key=lambda p:(p['group'],p['order_key'],p['ordinal']))]
    def find_violations(scan_roots=module.DEFAULT_SCAN_ROOTS):
        return scan_batch(suite,module,module.ROOT,module.iter_python_files(scan_roots),options=dict(lcFind='true'))
    def class_predicate(node):
        response=suite.native(CHECK,'_lc_class',[],dict(className=node.name))
        if len(response)>1 or any(r['code']!='LC_CLASS' for r in response):raise AssertionError('Invalid native class predicate')
        return bool(response)
    def project(entry,violations,options=None):
        payload=[dict(path=str(v.path),line=v.line,class_name=v.class_name,method_name=v.method_name) for v in violations]
        return suite.native(CHECK,entry,[dict(path='lifecycle-values.json',source=json.dumps(dict(values=payload),ensure_ascii=False))],dict(lcRoot=str(module.ROOT),**(options or {})))
    def errors(response):
        bad=[r['parameters'] for r in response if r['code']=='LC_PATH_ERROR']
        if bad:
            Path(min(bad,key=lambda p:p['ordinal'])['path']).relative_to(module.ROOT)
            raise AssertionError('Native path failure unexpectedly succeeded')
    def projected(entry,value):
        response=project(entry,[value]);errors(response)
        if len(response)!=1 or response[0]['code']!='LC_VALUE':raise AssertionError('Missing native value projection')
        return response[0]['parameters']
    def render(value):return projected('_lc_render',value)['rendered']
    def fact(value):return json.loads(projected('_lc_fact',value)['fact_json'])
    def cli_main(argv=None):
        parser=argparse.ArgumentParser(description=values('description')[0])
        parser.add_argument('paths',nargs='*',type=Path,help=values('paths_help')[0])
        parser.add_argument('--check',action='store_true',help=values('check_help')[0]);args=parser.parse_args(argv)
        roots=suite.native(CHECK,'_lc_roots',[],dict(requestedRoots=[str(p) for p in args.paths],defaultRoots=[str(p) for p in module.DEFAULT_SCAN_ROOTS]))
        if any(r['code']!='LC_ROOT' for r in roots):raise AssertionError('Unexpected native CLI root')
        selected=tuple(Path(p['value']) for p in sorted((r['parameters'] for r in roots),key=lambda p:p['ordinal']))
        violations=module.find_violations(selected)
        response=project('_lc_cli_report',violations,dict(checkMode=str(args.check).lower()))
        if any(r['code'] not in {'LC_OUTPUT','LC_EXIT','LC_PATH_ERROR'} for r in response):raise AssertionError('Unexpected native CLI output')
        for p in sorted((r['parameters'] for r in response if r['code']=='LC_OUTPUT'),key=lambda p:p['ordinal']):sys.stdout.write(p['stdout'])
        errors(response);status=[r['parameters']['value'] for r in response if r['code']=='LC_EXIT']
        if len(status)!=1:raise AssertionError('Missing native CLI status')
        return status[0]
    suite.replace(module,'iter_python_files',python_files)
    suite.replace(module,'find_violations',find_violations)
    suite.replace(module,'_is_versioned_write_transaction_class',class_predicate)
    suite.replace(module,'_violation_to_fact',fact)
    suite.replace(module.Violation,'render',render)
    suite.replace(module,'_violations_for_files',lambda root,files:scan_batch(suite,module,root,files))
    suite.replace(module,'cli_main',cli_main)
    if detectors:
        suite.replace(module,'scan_file',lambda path:scan_file(suite,module,'scan_file',path))
        suite.replace(module,'scan_file_content',lambda path,source:scan_file(suite,module,'scan_file_content',path,source))
