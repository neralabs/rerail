"""Transport CLI requests, filesystem observations, and native output rows."""
import argparse
import json
import os
from pathlib import Path
import sys
from allowlist_admin_port import install as install_admin

CHECK='conn_or_txn_union'


def one(rows,code,key):
    values=[r['parameters'][key] for r in rows if r['code']==code]
    if len(values)!=1:raise ValueError('One native '+code+' required')
    return values[0]


def emit(rows):
    for p in sorted((r['parameters'] for r in rows if r['code']=='CUT_CLI_OUTPUT'),key=lambda p:tuple(p[k] for k in ['phase','path_key','sign','length','digits','ordinal'])):
        sys.stdout.write(p['text'])


def install(backend,module,config,batch):
    discovery_globals=module.resolve_root_and_dirs.__globals__
    install_admin(backend,module,CHECK)
    def native(entry,options=None,files=None):
        return backend.native(CHECK,entry,files or [],options)
    def resolve(args,*,default_dirs=None):
        rows=native('_cut_cli_root',dict(cutCliHasRoot=str(int(args.root is not None)),cutCliRoot=args.root or '',cutCliDiscoverySource=str(Path(discovery_globals['__file__']).resolve()),cutCliHasDirs=str(int(args.dirs is not None)),cutCliDirs=list(args.dirs or []),cutCliHasDefaults=str(int(default_dirs is not None)),cutCliDefaults=list(default_dirs or [])))
        root=Path(one(rows,'CUT_CLI_ROOT','path')).resolve()
        arguments=[r['parameters']['path'] for r in sorted((r for r in rows if r['code']=='CUT_CLI_ARGUMENT'),key=lambda r:r['parameters']['ordinal'])]
        paths=list(root.iterdir()) if any(r['code']=='CUT_CLI_NEEDS_PUBLIC' for r in rows) else []
        rows=native('_cut_cli_expand',dict(cutCliRawDirs=arguments,cutDirectoryNames=[p.name for p in paths],cutDirectoryIsDir=[str(int(p.is_dir())) for p in paths],cutDirectoryExists=[str(int(p.exists())) for p in paths]))
        if any(r['code']!='CUT_CLI_DIRECTORY' for r in rows):raise ValueError('Unexpected native CLI directory')
        return root,[r['parameters']['path'] for r in sorted(rows,key=lambda r:tuple(r['parameters'][k] for k in ['argument','rank','path']))]
    def cli():
        schema=json.loads(config['cli_schema']);parser=argparse.ArgumentParser(description=schema['description'])
        for argument in schema['arguments']:
            kwargs=dict(argument['kwargs'])
            if 'type' in kwargs:kwargs['type']={'str':str}[kwargs['type']]
            parser.add_argument(argument['name'],**kwargs)
        args=parser.parse_args()
        rows=native('_cut_cli_start',dict(cutCliSign=str(int(bool(args.sign_allowlist))),cutCliSkipSignature=str(int(bool(args.skip_signature_check)))))
        action=one(rows,'CUT_CLI_ACTION','value')
        if action=='sign':
            ok=module.signing.sign_and_update_allowlist(Path(module.__file__),config['cli_linter'],sorted(module.ALLOWLIST),config['cli_signature_variable'])
            return one(native('_cut_cli_signed',dict(cutCliResult=str(int(bool(ok))))),'CUT_CLI_EXIT','status')
        if action=='verify':
            ok=module.signing.verify_allowlist(config['cli_linter'],sorted(module.ALLOWLIST),module._ALLOWLIST_SIGNATURE)
            rows=native('_cut_cli_verified',dict(cutCliResult=str(int(bool(ok)))));emit(rows)
            if any(r['code']=='CUT_CLI_EXIT' for r in rows):return one(rows,'CUT_CLI_EXIT','status')
            action=one(rows,'CUT_CLI_ACTION','value')
        if action!='resolve':raise ValueError('Unexpected native CLI action')
        root,directories=module.resolve_root_and_dirs(args,default_dirs=['public','conform'])
        rows=native('_cut_cli_requested',dict(cutCliPositional=list(args.paths or []),cutCliExpandedDirs=directories))
        requested=[r['parameters']['path'] for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
        paths=[];kinds=[];names=[];relative=[]
        for name in requested:
            path=root/name
            if not path.exists():continue
            if path.is_file():
                paths.append(path);kinds.append('direct');names.append(path.name);relative.append(path.name)
            else:
                for directory,_children,files in os.walk(path):
                    for filename in files:
                        child=Path(directory)/filename
                        paths.append(child);kinds.append('walk');names.append(filename);relative.append(child.relative_to(path).as_posix())
        rows=native('_cut_cli_files',dict(cutCliCandidates=[str(p) for p in paths],cutCliKinds=kinds,cutCliNames=names,cutCliRelative=relative))
        selected=[Path(r['parameters']['path']) for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
        if getattr(module.analyze_file,'_rerail_union_scanner',False):values=batch(backend,module,selected,root)
        else:
            values=[]
            for path in selected:values.extend(module.analyze_file(path,root=root))
        items=[dict(path=str(v.file_path),line=v.line_number,function=v.function_name,type=v.violation_type,details=v.details) for v in values]
        rows=native('_cut_cli_report',dict(cutCliCheck=str(int(bool(args.check)))),[dict(path='conn-cli-values.json',source=json.dumps(dict(items=items),ensure_ascii=False))])
        if any(r['code'] not in {'CUT_CLI_OUTPUT','CUT_CLI_EXIT'} for r in rows):raise ValueError('Unexpected native CLI report')
        emit(rows);return one(rows,'CUT_CLI_EXIT','status')
    backend.replace(module,'resolve_root_and_dirs',resolve)
    backend.replace(module,'cli_main',cli)
