"""Run the native CLI protocol; transport argparse values and callback results."""
import argparse
import json
from pathlib import Path
import sys

CHECK='dynamic_module_loading'


def one(rows,code,key):
    values=[r['parameters'][key] for r in rows if r['code']==code]
    if len(values)!=1:raise ValueError('One native '+code+' required')
    return values[0]


def emit(rows):
    known={'DML_CLI_OUTPUT','DML_CLI_ACTION','DML_CLI_EXIT','DML_CLI_ALLOW','DML_CLI_PATH_ERROR','DML_VERIFY'}
    if any(r['code'] not in known for r in rows):raise ValueError('Unexpected native CLI response')
    for p in sorted((r['parameters'] for r in rows if r['code']=='DML_CLI_OUTPUT'),key=lambda p:(p['phase'],p['ordinal'])):
        {'stdout':sys.stdout,'stderr':sys.stderr}[p['channel']].write(p['text'])


def install(backend,module):
    discovery_source=Path(module.resolve_root_and_dirs.__globals__['__file__']).resolve()
    config_rows=backend.native(CHECK,'_dynamic_cli_configuration',[])
    if not config_rows or any(r['code'] not in {'DML_CLI_CONFIG','DML_CLI_ARG'} for r in config_rows):raise ValueError('Missing native CLI schema')
    config={r['parameters']['key']:r['parameters']['value'] for r in config_rows if r['code']=='DML_CLI_CONFIG'}
    def resolve(args,*,default_dirs=None):
        rows=backend.native(CHECK,'_dynamic_cli_root',[],dict(cliHasRoot=str(int(args.root is not None)),cliRoot=args.root or '',discoverySource=str(discovery_source),cliHasDirs=str(int(args.dirs is not None)),cliDirs=list(args.dirs or []),cliHasDefaults=str(int(default_dirs is not None)),cliDefaultDirs=list(default_dirs or [])))
        if any(r['code'] not in {'DML_CLI_ROOT','DML_CLI_SCAN_ARG'} for r in rows):raise ValueError('Unexpected native root selection')
        root=Path(one(rows,'DML_CLI_ROOT','path')).resolve()
        arguments=sorted((r['parameters'] for r in rows if r['code']=='DML_CLI_SCAN_ARG'),key=lambda p:p['ordinal'])
        exports=['dml.paths=true','data-layer.scan.configured=true']+[f'data-layer.scan.{i}='+r['path'] for i,r in enumerate(arguments)]
        rows=backend.native(CHECK,'_dynamic_cli_paths',[],dict(env=exports),root=root)
        if any(r['code']!='DML_CLI_DIRECTORY' for r in rows):raise ValueError('Unexpected native directory expansion')
        return root,[r['parameters']['path'] for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
    def cli():
        parser=argparse.ArgumentParser(description=config['description'])
        for row in sorted((r['parameters'] for r in config_rows if r['code']=='DML_CLI_ARG'),key=lambda p:p['ordinal']):
            schema=json.loads(row['schema'])
            if 'type' in schema:schema['type']={'str':str}[schema['type']]
            parser.add_argument(row['flag'],**schema)
        args=parser.parse_args()
        rows=backend.native(CHECK,'_dynamic_cli_start',[],dict(cliSign=str(int(bool(args.sign_allowlist))),cliAllowed=list(module.ALLOWLISTED_FILES)))
        emit(rows);action=one(rows,'DML_CLI_ACTION','value')
        entries=[r['parameters']['value'] for r in sorted((r for r in rows if r['code']=='DML_CLI_ALLOW'),key=lambda r:r['parameters']['ordinal'])]
        if action=='sign':
            result=module.sign_and_update_allowlist(script_path=Path(module.__file__),linter_name=module.LINTER_NAME,entries=entries,variable_name=config['signature_variable'])
            rows=backend.native(CHECK,'_dynamic_cli_signed',[],dict(cliResult=str(int(bool(result)))));emit(rows);return one(rows,'DML_CLI_EXIT','status')
        if action!='verify':raise ValueError('Unexpected native CLI action')
        result=module.verify_allowlist(module.LINTER_NAME,entries,module.ALLOWLIST_SIGNATURE)
        rows=backend.native(CHECK,'_dynamic_cli_verified',[],dict(cliResult=str(int(bool(result)))));emit(rows)
        if any(r['code']=='DML_CLI_EXIT' for r in rows):return one(rows,'DML_CLI_EXIT','status')
        if one(rows,'DML_CLI_ACTION','value')!='resolve':raise ValueError('Unexpected native post-verification action')
        root,directories=module.resolve_root_and_dirs(args,default_dirs=['public'])
        emit(backend.native(CHECK,'_dynamic_cli_begin',[]))
        values=module.find_violations(root,directories)
        options={key:[str(v[i]) for v in values] for i,key in enumerate(['cliFile','cliLine','cliSnippet','cliDescription'])}
        options.update(sourceRoot=str(root),cliCheck=str(int(bool(args.check))))
        rows=backend.native(CHECK,'_dynamic_cli_report',[],options);emit(rows)
        errors=[r['parameters'] for r in rows if r['code']=='DML_CLI_PATH_ERROR']
        if errors:
            if len(errors)!=1:raise ValueError('Multiple first native path errors')
            Path(errors[0]['path']).relative_to(root)
            raise ValueError('Native path error unexpectedly succeeded')
        return one(rows,'DML_CLI_EXIT','status')
    def verify(linter_name,entries,signature_b64,*,version=1,warn_only=False):
        rows=backend.native(CHECK,'_dynamic_verify',[],dict(verifyLinter=linter_name,verifyEntries=list(entries),verifySignature=signature_b64,verifyVersion=str(version),verifyWarn=str(int(bool(warn_only)))))
        emit(rows);return bool(one(rows,'DML_VERIFY','ok'))
    def sign(script_path,linter_name,entries,variable_name,*,version=1):
        rows=backend.native(CHECK,'_dynamic_sign_request',[],dict(signLinter=linter_name,signEntries=list(entries),signVersion=str(version)))
        if len(rows)!=1 or rows[0]['code']!='DML_SIGN_REQUEST':raise ValueError('One native signing request required')
        p=rows[0]['parameters'];reply=backend.sign_ed25519([dict(id=p['request_id'],key_env=p['key_env'],message=p['message'])])
        if len(reply)!=1 or reply[0]['id']!=p['request_id']:raise ValueError('Mismatched native signing reply')
        result=reply[0];path=Path(script_path)
        if result['status']=='signed':rows=backend.native(CHECK,'_dynamic_sign_update',[],dict(scriptSource=path.read_text(encoding='utf-8'),scriptPath=str(path),newSignature=result['signature'],signVariable=variable_name))
        else:rows=backend.native(CHECK,'_dynamic_sign_error',[],dict(signStatus=result['status'],signDetail=result['detail']))
        if len(rows)!=1 or rows[0]['code']!='DML_SIGN_RESULT':raise ValueError('One native signing result required')
        value=rows[0]['parameters']
        if value['write_file']:path.write_text(value['updated_source'],encoding='utf-8')
        sys.stdout.write(value['stdout']);sys.stderr.write(value['stderr']);return bool(value['ok'])
    backend.replace(module,'resolve_root_and_dirs',resolve)
    backend.replace(module,'cli_main',cli);backend.replace(module,'verify_allowlist',verify);backend.replace(module,'sign_and_update_allowlist',sign)
