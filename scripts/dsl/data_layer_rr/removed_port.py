"""RemovedCompat file I/O, argparse and passive DTO transport."""
import json
from pathlib import Path

CHECK='removed_compat_surfaces'


def scan_batch(suite,module,paths):
    files=[]; owners={}
    for index,path in enumerate(paths):
        try: source=path.read_text(encoding='utf-8')
        except OSError: continue
        key=f'file-{index}.py'; owners[key]=(index,str(path))
        files.append(dict(path=key,source=source))
    rows=suite.native(CHECK,'scan_file',files,dict(sourceRepoRoot=str(module._REPO_ROOT),
        ownerDocs=[f['path'] for f in files],ownerPaths=[owners[f['path']][1] for f in files]))
    if any(r['code']!='RCS001' for r in rows): raise AssertionError('Unexpected native scanner row')
    rows.sort(key=lambda r:(owners[r['path']][0],r['parameters']['ordinal']))
    return tuple(module.Violation(r['parameters']['result_path'],r['line'],r['code'],r['message']) for r in rows)


def install(suite,module,detectors=False):
    import argparse
    rows=suite.native(CHECK,'_rcs_configuration',[])
    if not rows or any(r['code']!='RCS_CONFIG' for r in rows): raise AssertionError('Missing native RemovedCompat configuration')
    def values(key):
        return [p['value'] for p in sorted((r['parameters'] for r in rows if r['parameters']['key']==key),key=lambda p:p['ordinal'])]
    suite.replace(module,'_FORBIDDEN_NAMES',frozenset(values('names')))
    suite.replace(module,'_DEFAULT_ROOTS',tuple(values('roots')))
    data=json.loads(values('extractor')[0])
    for key in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']: data[key]=tuple(data[key])
    suite.replace(module,'REMOVED_COMPAT_SURFACES_EXTRACTOR',type(module.REMOVED_COMPAT_SURFACES_EXTRACTOR)(**data))
    if hasattr(suite,'configuration_bindings'):
        suite.configuration_bindings[CHECK]=dict(nativeCall=len(suite.calls)-1,names=['_FORBIDDEN_NAMES','_DEFAULT_ROOTS','REMOVED_COMPAT_SURFACES_EXTRACTOR'])
    def python_files(paths):
        roots=suite.native(CHECK,'_rcs_roots',[],dict(sourceRepoRoot=str(module._REPO_ROOT),requestedRoots=paths))
        if any(r['code']!='RCS_ROOT' for r in roots): raise AssertionError('Unexpected native root')
        candidates=[]
        for row in sorted(roots,key=lambda r:r['parameters']['ordinal']):
            root=Path(row['parameters']['value'])
            if root.is_file(): candidates.append(str(root))
            elif root.is_dir(): candidates.extend(str(p) for p in root.rglob('*'))
        selected=suite.native(CHECK,'_rcs_files',[],dict(candidatePaths=candidates))
        if any(r['code']!='RCS_FILE' for r in selected): raise AssertionError('Unexpected native file')
        return tuple(Path(r['parameters']['value']) for r in sorted(selected,key=lambda r:r['parameters']['order_key']))
    def scan(paths): return scan_batch(suite,module,module._python_files(paths))
    def cli_main():
        import sys
        parser=argparse.ArgumentParser(description=values('description')[0])
        parser.add_argument('paths',nargs='*'); parser.add_argument('--check',action='store_true')
        parser.add_argument('--json',action='store_true'); parser.add_argument('--json-details',action='store_true')
        args=parser.parse_args(); violations=module.scan(args.paths)
        options=dict(cliPath=[v.path for v in violations],cliLine=[str(v.line) for v in violations],
            cliLineJson=[json.dumps(v.line) for v in violations],cliCode=[v.code for v in violations],
            cliMessage=[v.message for v in violations],checkMode=str(args.check).lower(),
            jsonMode=str(args.json).lower(),jsonDetailsMode=str(args.json_details).lower())
        response=suite.native(CHECK,'_rcs_cli_report',[],options)
        if any(r['code'] not in {'RCS_OUTPUT','RCS_EXIT'} for r in response): raise AssertionError('Unexpected native CLI row')
        for p in sorted((r['parameters'] for r in response if r['code']=='RCS_OUTPUT'),key=lambda p:(p['phase'],p['ordinal'])):
            sys.stdout.write(p['stdout'])
        exits=[r['parameters']['value'] for r in response if r['code']=='RCS_EXIT']
        if len(exits)!=1: raise AssertionError('Missing native CLI status')
        return exits[0]
    suite.replace(module,'_python_files',python_files)
    suite.replace(module,'scan',scan)
    suite.replace(module,'cli_main',cli_main)
    if detectors: suite.replace(module,'scan_file',lambda path:scan_batch(suite,module,[path]))
