"""Record complete admin CLI output, service calls, status and signed script bytes."""
import base64
import contextlib
import io
import os
from pathlib import Path
import sys
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from replay import write_fixture
from admin_public_contract import CHECK,BAD
from impl_direct_execution_ratchet_contract import environment


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    rows=[dict(id='empty',files=[]),dict(id='empty-check',files=[],args=['--check']),
        dict(id='positive',files=[file('l8000_admin/tools/a.py')]),dict(id='positive-check',files=[file('l8000_admin/tools/a.py')],args=['--check']),
        dict(id='multiple-files',files=[file('l8000_admin/tools/z.py'),file('l8000_admin/tools/a.py')]),
        dict(id='docstring',files=[file('l8000_admin/tools/a.py','"""\n'+BAD+'"""\n')]),
        dict(id='skip-paths',files=[file('l8000_admin/tools/test_a.py'),file('l8000_admin/tools/tests/a.py')]),
        dict(id='allowlisted',files=[file('l8000_admin/tools/a.py')],allowed=['l8000_admin/tools/a.py'],verifyResult=True),
        dict(id='custom-directory',files=[file('other/a.py')],dirs=['other']),
        dict(id='explicit-file',files=[file('other/a.txt')],dirs=['other/a.txt']),
        dict(id='empty-directories',files=[file('l8000_admin/tools/a.py')],dirs=[]),
        dict(id='duplicate-targets',files=[file('l8000_admin/tools/a.py')],dirs=['l8000_admin/tools','l8000_admin/tools/a.py']),
        dict(id='help',files=[],args=['--help']),dict(id='unknown-option',files=[],args=['--unknown']),
        dict(id='verification-failure',files=[file('l8000_admin/tools/a.py')],verifyResult=False),
        dict(id='verification-streams',files=[],verifyResult=False,verifyStdout='verification output\n',verifyStderr='verification error\n'),
        dict(id='verification-arguments',files=[],allowed=['z.py','a.py','a.py'],verifyResult=True)]
    for i,signature in enumerate(['','UNSIGNED','NEEDS_SIGNING','!!!!']):rows.append(dict(id=f'signature-{i}',files=[],signature=signature))
    for ok in [False,True]:rows.append(dict(id='sign-callback-'+str(ok),files=[],args=['--sign-allowlist'],signResult=ok))
    rows.extend([
        dict(id='sign-before-check',files=[file('l8000_admin/tools/a.py')],args=['--sign-allowlist','--check'],signResult=True),
        dict(id='sign-no-key',files=[],args=['--sign-allowlist']),
        dict(id='sign-key-length',files=[],args=['--sign-allowlist'],key='00'),
        dict(id='sign-invalid-hex',files=[],args=['--sign-allowlist'],key='not-hex'),
        dict(id='sign-public-fixture',files=[],args=['--sign-allowlist'],key='00'*32),
        dict(id='sign-sentinel',files=[],args=['--sign-allowlist'],key='00'*32,script='ALLOWLIST_SIGNATURE: str = NEEDS_SIGNING # pending\n'),
        dict(id='sign-duplicates',files=[],args=['--sign-allowlist'],key='00'*32,script='ALLOWLIST_SIGNATURE = "a"\nALLOWLIST_SIGNATURE: str = "b" # retained\n'),
        dict(id='sign-missing-assignment',files=[],args=['--sign-allowlist'],key='00'*32,script='OTHER = "value"\n'),
    ])
    return [dict(kind='cli',**row) for row in rows]


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    script=root/'scripts/lint_admin_async_purity.py';script.parent.mkdir()
    script.write_text(case.get('script',Path(module.__file__).read_text()))
    before=base64.b64encode(script.read_bytes()).decode();out=io.StringIO();err=io.StringIO();events=[]
    def verify(*args,**kwargs):
        events.append(dict(name='verify',args=canonical(args,scratch),kwargs=canonical(kwargs,scratch)))
        print(case.get('verifyStdout',''),end='');err.write(case.get('verifyStderr',''));return case['verifyResult']
    def sign(*args,**kwargs):
        events.append(dict(name='sign',args=canonical(args,scratch),kwargs=canonical(kwargs,scratch)));return case['signResult']
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.dict(os.environ,environment(case),clear=True));stack.enter_context(contextlib.chdir(root))
        stack.enter_context(patch.object(module,'__file__',str(script)));stack.enter_context(patch.object(module,'SCAN_DIRS',case.get('dirs',['l8000_admin/tools'])))
        stack.enter_context(patch.object(module,'ALLOWLISTED_FILES',case.get('allowed',[])))
        stack.enter_context(patch.object(sys,'argv',['lint_admin_async_purity.py',*case.get('args',[])]))
        if 'signature' in case:stack.enter_context(patch.object(module,'ALLOWLIST_SIGNATURE',case['signature']))
        if 'verifyResult' in case:stack.enter_context(patch.object(module.lint_signing,'verify_allowlist',verify))
        if 'signResult' in case:stack.enter_context(patch.object(module.lint_signing,'sign_and_update_allowlist',sign))
        with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            try:value=dict(status=module.cli_main())
            except BaseException as error:value=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    return dict(value=value,stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'),events=events,
        scriptBefore=before,scriptAfter=base64.b64encode(script.read_bytes()).decode())


def install(backend,module,detectors=False):
    from admin_components_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.admin-cli/v1',cases,exercise,install,['admin_cli_contract.py','admin_public_contract.py','impl_direct_execution_ratchet_contract.py'],guard_modules=['lint_lib.signing','scripts.lint_lib.signing'])
