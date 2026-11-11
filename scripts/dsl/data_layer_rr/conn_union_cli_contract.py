"""Record CLI streams, callbacks, errors, and fixture-only signature updates."""
import contextlib
import importlib
import io
import os
from pathlib import Path
import sys
from unittest.mock import patch
from contract_runner import main
from conn_union_contract import CHECK,UNION
from dto import canonical
from replay import write_fixture


def cases(_module):
    def file(path,source=UNION):return dict(path=path,source=source)
    rows=[dict(id='clean',files=[]),dict(id='positive',files=[file('impl/a.py')]),
        dict(id='positive-check',files=[file('impl/a.py')],arguments=['--check']),
        dict(id='separate',files=[file('impl/a.py','def load(conn: Connection,txn: Transaction):\n pass\n')]),
        dict(id='roots',files=[file(p+'/a.py') for p in ['api','impl','data_layer','conform','l1000_a']]),
        dict(id='legacy-roots',files=[file('api/a.py'),file('impl/a.py')]),
        dict(id='default-root',files=[file('impl/a.py')],noRoot=True),
        dict(id='dirs-empty',files=[file('impl/a.py')],arguments=['--dirs']),
        dict(id='dirs-tests',files=[file('tests/a.py')],arguments=['--dirs','tests']),
        dict(id='nested',files=[file('impl/'+p+'/a.py') for p in ['normal','tests','test','__pycache__','tmp','.git','.hidden']]),
        dict(id='suffixes',files=[file('impl/'+p) for p in ['.py','a.py','a.pyi','a.PY','test_a.py','a_test.py']]),
        dict(id='path-order',files=[file('impl/'+p) for p in ['z.py','a/z.py','a-.py','a.py']]),
        dict(id='duplicates',files=[file('impl/a.py',UNION+UNION)]),
        dict(id='duplicate-paths',files=[file('impl/a.py')],arguments=['impl','impl']),
        dict(id='direct-test-name',files=[file('impl/test_a.py')],arguments=['impl/test_a.py']),
        dict(id='direct-non-python',files=[file('a.txt')],arguments=['a.txt']),
        dict(id='missing-path',files=[],arguments=['missing.py']),
        dict(id='file-link',files=[file('impl/a.py'),dict(path='impl/link.py',symlink='a.py')]),
        dict(id='directory-link',files=[file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')]),
        dict(id='callback-findings',files=[file('impl/z.py'),file('impl/a.py')],callback='findings'),
        dict(id='callback-empty',files=[file('impl/a.py')],callback='empty'),
        dict(id='unsigned',files=[file('impl/a.py')],signature='UNSIGNED'),
        dict(id='bad-signature',files=[file('impl/a.py')],signature='bad'),
        dict(id='skip-signature',files=[file('impl/a.py')],signature='UNSIGNED',arguments=['--skip-signature-check','--check']),
        dict(id='verify-callback-false',files=[file('impl/a.py')],verify=False),
        dict(id='verify-callback-true',files=[file('impl/a.py')],verify=True,signature='UNSIGNED'),
        dict(id='sign-callback-true',files=[],arguments=['--sign-allowlist'],sign=True),
        dict(id='sign-callback-false',files=[],arguments=['--sign-allowlist'],sign=False),
        dict(id='sign-missing-key',files=[],arguments=['--sign-allowlist'],signKey='missing'),
        dict(id='sign-bad-key',files=[],arguments=['--sign-allowlist'],signKey='bad-hex'),
        dict(id='sign-short-key',files=[],arguments=['--sign-allowlist'],signKey='short'),
        dict(id='sign-fixture-key',files=[],arguments=['--sign-allowlist'],signKey='fixture'),
        dict(id='sign-missing-variable',files=[],arguments=['--sign-allowlist'],signKey='fixture',script='OTHER = "OLD"\n'),
        dict(id='sign-single-quoted',files=[],arguments=['--sign-allowlist'],signKey='fixture',script="_ALLOWLIST_SIGNATURE = 'OLD'\n"),
        dict(id='parser-help',files=[],arguments=['--help']),dict(id='parser-bad-option',files=[],arguments=['--bad-option'])]
    return [dict(kind='cli',**row) for row in rows]


def exercise(module,case,scratch):
    if 'NERA_SIGNING_KEY' in os.environ:raise AssertionError('Run this fixture worker with env -u NERA_SIGNING_KEY')
    scratch=scratch.resolve()
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    discovery=importlib.import_module('scripts.lint_lib.discovery')
    discovery_path=root/'scripts/lint_lib/discovery.py';discovery_path.parent.mkdir(parents=True,exist_ok=True);discovery_path.write_text('pass\n')
    script=root/'cli_owner.py';script.write_text(case.get('script','_ALLOWLIST_SIGNATURE: str = UNSIGNED\n'))
    calls=[];out=io.StringIO();err=io.StringIO();result={}
    argv=['lint_conn_or_txn_union.py',*([] if case.get('noRoot') else ['--root',str(root)]),*case.get('arguments',[])]
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.object(module,'__file__',str(script)));stack.enter_context(patch.object(discovery,'__file__',str(discovery_path)))
        stack.enter_context(patch.object(module,'_ALLOWLIST_SIGNATURE',case.get('signature',module._ALLOWLIST_SIGNATURE)))
        stack.enter_context(patch.object(sys,'argv',argv));stack.enter_context(contextlib.redirect_stdout(out));stack.enter_context(contextlib.redirect_stderr(err))
        if 'callback' in case:
            def analyze(path,*,root=None):
                calls.append(dict(entry='analyze_file',path=path,root=root))
                return [] if case['callback']=='empty' else [module.Violation(path,2,'fixture','UNION','fixture details')]
            stack.enter_context(patch.object(module,'analyze_file',analyze))
        if 'verify' in case:
            def verify(*args,**kwargs):calls.append(dict(entry='verify',args=args,kwargs=kwargs));return case['verify']
            stack.enter_context(patch.object(module.signing,'verify_allowlist',verify))
        if 'sign' in case:
            def sign(*args,**kwargs):calls.append(dict(entry='sign',args=args,kwargs=kwargs));return case['sign']
            stack.enter_context(patch.object(module.signing,'sign_and_update_allowlist',sign))
        key={'bad-hex':'xx','short':'aa','fixture':bytes(range(32)).hex()}.get(case.get('signKey'))
        if key is not None:stack.enter_context(patch.dict(os.environ,{'NERA_SIGNING_KEY':key}))
        try:
            try:status=module.cli_main();raised=None
            except SystemExit as error:status=error.code;raised='SystemExit'
            result.update(status=status,raised=raised)
        except Exception as error:result['error']=dict(type=type(error).__name__,args=canonical(error.args,scratch))
    result.update(stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'),calls=canonical(calls,scratch),scriptAfter=script.read_text())
    return result


def install(backend,module,detectors=False):
    from conn_union_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':
    main(CHECK,'rerail.conn-union-cli/v1',cases,exercise,install,['conn_union_cli_contract.py','conn_union_port.py'],guard_modules=('scripts.lint_lib.signing',))
