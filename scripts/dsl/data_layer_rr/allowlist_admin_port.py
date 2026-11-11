"""Transport explicit native allowlist operations and perform requested file I/O."""
from pathlib import Path
from types import SimpleNamespace
import sys


def install(backend,module,check):
    proxy=SimpleNamespace(**vars(module.signing))
    def emit(rows):
        for row in rows:
            if row['code']=='ADMIN_OUTPUT':
                p=row['parameters'];{'stdout':sys.stdout,'stderr':sys.stderr}[p['channel']].write(p['text'])
    def verify(linter_name,entries,signature_b64,*,version=1,warn_only=False):
        rows=backend.native(check,'_admin_verify',[],dict(verifyLinter=linter_name,verifyEntries=list(entries),verifySignature=signature_b64,verifyVersion=str(version),verifyWarn=str(int(bool(warn_only)))))
        if any(r['code'] not in {'ADMIN_VERIFY','ADMIN_OUTPUT'} for r in rows):raise ValueError('Unexpected native allowlist verification')
        emit(rows);values=[r['parameters']['ok'] for r in rows if r['code']=='ADMIN_VERIFY']
        if len(values)!=1:raise ValueError('One native allowlist verification result required')
        return bool(values[0])
    def sign(script_path,linter_name,entries,variable_name,*,version=1):
        rows=backend.native(check,'_admin_sign_request',[],dict(signLinter=linter_name,signEntries=list(entries),signVersion=str(version)))
        if len(rows)!=1 or rows[0]['code']!='ADMIN_SIGN_REQUEST':raise ValueError('One native allowlist signing request required')
        p=rows[0]['parameters'];reply=backend.sign_ed25519([dict(id=p['request_id'],key_env=p['key_env'],message=p['message'])])
        if len(reply)!=1 or reply[0]['id']!=p['request_id']:raise ValueError('Mismatched native allowlist signing reply')
        value=reply[0];path=Path(script_path)
        if value['status']=='signed':rows=backend.native(check,'_admin_sign_update',[],dict(scriptSource=path.read_text(encoding='utf-8'),scriptPath=str(path),newSignature=value['signature'],signVariable=variable_name))
        else:rows=backend.native(check,'_admin_sign_error',[],dict(signStatus=value['status'],signDetail=value['detail']))
        if len(rows)!=1 or rows[0]['code']!='ADMIN_SIGN_RESULT':raise ValueError('One native allowlist signing result required')
        p=rows[0]['parameters']
        if p['write_file']:path.write_text(p['updated_source'],encoding='utf-8')
        sys.stdout.write(p['stdout']);sys.stderr.write(p['stderr']);return bool(p['ok'])
    proxy.verify_allowlist=verify;proxy.sign_and_update_allowlist=sign
    backend.replace(module,'signing',proxy)
