"""Transport argparse inputs and explicit services; native rules control admin CLI decisions."""
import argparse
from dataclasses import fields
import json
from pathlib import Path
import sys
from allowlist_admin_port import install as install_signing

CHECK='admin_async_purity'


def one(rows,code,key):
    values=[r['parameters'][key] for r in rows if r['code']==code]
    if len(values)!=1:raise ValueError('One native '+code+' required')
    return values[0]


def emit(rows):
    for row in sorted((r['parameters'] for r in rows if r['code']=='AP_CLI_OUTPUT'),key=lambda p:(p['phase'],p['ordinal'],p['part'])):sys.stdout.write(row['text'])


def install(backend,module,config):
    install_signing(backend,module,CHECK,signing_attribute='lint_signing')
    def native(entry,options=None,files=None):return backend.native(CHECK,entry,files or [],options or {})
    def cli():
        schema=config['cli_schema'];parser=argparse.ArgumentParser(description=schema['description'])
        for argument in schema['arguments']:parser.add_argument(*argument['names'],**argument['kwargs'])
        args=parser.parse_args()
        rows=native('_ap_cli_start',dict(apCliSign=str(int(bool(args.sign_allowlist))),apCliSource=str(Path(module.__file__).resolve()),apCliAllowed=list(module.ALLOWLISTED_FILES)))
        root=Path(one(rows,'AP_CLI_ROOT','path'))
        allowed=[r['parameters']['value'] for r in sorted((r for r in rows if r['code']=='AP_CLI_ALLOW'),key=lambda r:(r['parameters']['value'],r['parameters']['ordinal']))]
        action=one(rows,'AP_CLI_ACTION','value')
        if action=='sign':
            ok=module.lint_signing.sign_and_update_allowlist(Path(module.__file__),module.LINTER_NAME,allowed,'ALLOWLIST_SIGNATURE')
            return one(native('_ap_cli_signed',dict(apCliResult=str(int(bool(ok))))),'AP_CLI_EXIT','status')
        if action!='verify':raise ValueError('Unexpected native admin CLI action')
        ok=module.lint_signing.verify_allowlist(module.LINTER_NAME,allowed,module.ALLOWLIST_SIGNATURE)
        rows=native('_ap_cli_verified',dict(apCliResult=str(int(bool(ok)))));emit(rows)
        if any(r['code']=='AP_CLI_EXIT' for r in rows):return one(rows,'AP_CLI_EXIT','status')
        if one(rows,'AP_CLI_ACTION','value')!='scan':raise ValueError('Unexpected native admin verification action')
        violations=module._collect_violations(root)
        values=[{f.name:getattr(v,f.name) for f in fields(v)} for v in violations]
        rows=native('_ap_cli_report',dict(apCliCheck=str(int(bool(args.check)))),[dict(path='admin-cli-values.json',source=json.dumps(dict(rows=values),ensure_ascii=False,allow_nan=False))])
        if any(r['code'] not in {'AP_CLI_OUTPUT','AP_CLI_EXIT'} for r in rows):raise ValueError('Unexpected native admin CLI output')
        emit(rows);return one(rows,'AP_CLI_EXIT','status')
    backend.replace(module,'cli_main',cli)
