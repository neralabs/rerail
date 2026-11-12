"""Transport boundary CLI parsing and I/O; native relations choose roots, actions and results."""
import argparse
import importlib
from pathlib import Path
import sys
from native_ratchet import NativeRatchet

CHECK='async_sync_boundary'


def install(backend,module,config):
    def query(entry,files=(),options=None,*,root=None):return backend.native(CHECK,entry,list(files),options or {},root=root)
    discovery=module.resolve_root_and_dirs.__globals__
    ratchet=NativeRatchet(backend,CHECK,importlib.import_module('lint_lib.ratchet'));ratchet.install()
    backend.replace(module,'check_ratchet',ratchet.check_baseline)
    backend.replace(module,'update_ratchet_baseline',ratchet.update)
    def resolve(args,default_dirs=None):
        rows=query('_bs_cli_root',options=dict(bsLegacyPaths=str(int(bool(args.path))),bsLegacyDirs=list(args.path or []),bsModulePath=str(Path(module.__file__).resolve()),bsExplicitRoot=str(int(args.root is not None)),bsRoot=args.root or '',bsDiscoverySource=str(Path(discovery['__file__']).resolve()),bsHasDirs=str(int(args.dirs is not None)),bsDirs=list(args.dirs or []),bsDefaults=list(default_dirs or [])))
        roots=[r['parameters'] for r in rows if r['code']=='BS_CLI_ROOT']
        if len(roots)!=1 or any(r['code'] not in {'BS_CLI_ROOT','BS_CLI_ARG'} for r in rows):raise ValueError('One native boundary CLI root required')
        root=Path(roots[0]['path']).resolve();paths=[r['parameters']['path'] for r in sorted((r for r in rows if r['code']=='BS_CLI_ARG'),key=lambda r:r['parameters']['ordinal'])]
        if roots[0]['direct']:return root,paths
        rows=query('_bs_cli_dirs',options=dict(env=['boundary.paths=true',*[f'data-layer.scan.{i}={p}' for i,p in enumerate(paths)]]),root=root)
        if any(r['code']!='BS_CLI_DIR' for r in rows):raise ValueError('Unexpected native boundary CLI directory')
        return root,[r['parameters']['path'] for r in sorted(rows,key=lambda r:(r['parameters']['argument'],r['parameters']['rank'],r['parameters']['path']))]
    backend.replace(module,'resolve_root_and_dirs',resolve)
    def cli():
        schema=config['cli_schema'];parser=argparse.ArgumentParser(description=schema['description'])
        for item in schema['arguments']:
            kwargs=dict(item['kwargs'])
            if 'type' in kwargs:kwargs['type']={'str':str}[kwargs['type']]
            parser.add_argument(*item['names'],**kwargs)
        args=parser.parse_args();root,paths=module.resolve_root_and_dirs(args,default_dirs=['public','conform'])
        result=module.scan_tree(root,paths)
        actionable,_=module.print_results(result,show_context=args.context,show_allowlisted=args.show_allowlisted,root=root)
        options=dict(bsActionable=str(actionable),bsUpdate=str(int(args.update_baseline)),bsCheck=str(int(args.check)))
        rows=query('_bs_cli_control',options=options)
        if len(rows)!=1 or rows[0]['code']!='BS_CLI_CONTROL':raise ValueError('One native boundary CLI action required')
        p=rows[0]['parameters'];passed=True
        if p['action']=='update':module.update_ratchet_baseline('lint_async_sync_boundary',dict(actionable=p['count']),sign=args.sign,reason=args.reason)
        elif p['action']=='check':passed=module.check_ratchet('lint_async_sync_boundary',dict(actionable=p['count']),require_signature=not args.skip_signature_check)
        elif p['action']!='done':raise ValueError('Unknown native boundary CLI action')
        rows=query('_bs_cli_finish',options=dict(options,bsCheckPassed=str(int(passed))))
        if any(r['code'] not in {'BS_CLI_OUTPUT','BS_CLI_RETURN'} for r in rows):raise ValueError('Unexpected boundary CLI result')
        for row in rows:
            if row['code']=='BS_CLI_OUTPUT':sys.stdout.write(row['parameters']['text'])
        results=[r['parameters']['status'] for r in rows if r['code']=='BS_CLI_RETURN']
        if len(results)!=1:raise ValueError('One boundary CLI status required')
        return results[0]
    backend.replace(module,'cli_main',cli)
