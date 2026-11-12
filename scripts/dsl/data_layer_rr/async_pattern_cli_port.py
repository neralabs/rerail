"""Marshal async CLI arguments, filesystem demand batches, and native output actions."""
import argparse
import importlib
import json
from pathlib import Path
import sys
from dataclasses import fields
from async_pattern_files_port import directories
from native_ratchet import NativeRatchet

CHECK='async_patterns'


def install(backend,module,config):
    def query(entry,files=(),options=None,*,root=None):return backend.native(CHECK,entry,list(files),options or {},root=root)
    def one(rows,code,key):
        values=[r['parameters'][key] for r in rows if r['code']==code]
        if len(values)!=1:raise ValueError('One native '+code+' required')
        return values[0]
    discovery=module.resolve_root_and_dirs.__globals__
    ratchet=NativeRatchet(backend,CHECK,importlib.import_module('lint_lib.ratchet'));ratchet.install()
    def resolve(args,*,default_dirs=None):
        rows=query('_ap_cli_root',options=dict(apExplicitRoot=str(int(args.root is not None)),apRoot=args.root or '',apDiscoverySource=str(Path(discovery['__file__']).resolve()),apHasDirs=str(int(args.dirs is not None)),apDirs=list(args.dirs or []),apDefaults=list(default_dirs or [])))
        root=Path(one(rows,'AP_CLI_ROOT','path')).resolve();raw=[r['parameters']['path'] for r in sorted((r for r in rows if r['code']=='AP_CLI_ARG'),key=lambda r:r['parameters']['ordinal'])]
        rows=query('_ap_cli_dirs',options=dict(env=['async-pattern.paths=true',*[f'data-layer.scan.{i}={p}' for i,p in enumerate(raw)]]),root=root)
        if any(r['code']!='AP_CLI_DIR' for r in rows):raise ValueError('Unexpected native async CLI directory')
        return root,[r['parameters']['path'] for r in sorted(rows,key=lambda r:(r['parameters']['argument'],r['parameters']['rank'],r['parameters']['path']))]
    backend.replace(module,'resolve_root_and_dirs',resolve)
    def emit(rows):
        for p in sorted((r['parameters'] for r in rows if r['code']=='AP_CLI_OUTPUT'),key=lambda p:tuple(p[k] for k in ['phase','path_key','section','line','group','ordinal','sub'])):sys.stdout.write(p['text'])
    def cli():
        schema=config['cli_schema'];parser=argparse.ArgumentParser(description=schema['description'])
        for item in schema['arguments']:
            kwargs=dict(item['kwargs'])
            if 'type' in kwargs:kwargs['type']={'str':str}[kwargs['type']]
            parser.add_argument(*item['names'],**kwargs)
        args=parser.parse_args();root,dirs=module.resolve_root_and_dirs(args,default_dirs=module.SCAN_PATHS)
        emit(query('_ap_cli_header'))
        result=directories(backend,module,root,dirs);module.lint_api_transactions(root)
        raw=dict(files_checked=result.files_checked,files_with_violations=result.files_with_violations,violations=[{f.name:(str(v.file) if f.name=='file' else getattr(v,f.name)) for f in fields(v)} for v in result.violations])
        rows=query('_ap_cli_report',[dict(path='async-pattern-cli.json',source=json.dumps(dict(groups=[raw]),ensure_ascii=False))],dict(apCliRoot=str(root),apSummary=str(int(args.summary)),apActionable=str(int(args.actionable)),apVerbose=str(int(args.verbose)),apCheck=str(int(args.check)),apUpdate=str(int(args.update_baseline))))
        if any(r['code'] not in {'AP_CLI_OUTPUT','AP_CLI_CONTROL'} for r in rows):raise ValueError('Unexpected async CLI result')
        emit(rows);controls=[r['parameters'] for r in rows if r['code']=='AP_CLI_CONTROL']
        if len(controls)!=1:raise ValueError('One native async CLI action required')
        p=controls[0];counts=dict(violations=p['count'])
        if p['action']=='update':ratchet.namespace.update_ratchet_baseline('lint_async_patterns',counts,sign=args.sign,reason=args.reason);return 0
        if p['action']=='check':return 0 if ratchet.namespace.check_ratchet('lint_async_patterns',counts,require_signature=not args.skip_signature_check) else 1
        if p['action']!='done':raise ValueError('Unknown native async CLI action')
        return 0
    backend.replace(module,'cli_main',cli)
