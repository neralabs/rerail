"""Record actual CLI signing with public synthetic keys and complete file bytes."""
from contextlib import redirect_stdout,redirect_stderr,ExitStack
import base64
import io
import os
from pathlib import Path
import sys
from unittest.mock import patch
from dto import canonical
from dispatch_signing_contract import CASES as COMMON_CASES
from impl_direct_execution_signing_contract import fixture_environment,normalize

CHECK='dynamic_module_loading'


def cases(_module):
    result=[]
    for item in COMMON_CASES:
        c=dict(item,kind='signing')
        if 'before' in c:c['before']=c['before'].replace('INVENTORY_SIGNATURE','ALLOWLIST_SIGNATURE')
        result.append(c)
    result.extend([
        dict(kind='signing',id='unicode-duplicates',key='00'*32,entries=['z','a','a','é','😀']),
        dict(kind='signing',id='check-precedence',key='01'*32,args=['--check','--sign-allowlist']),
        dict(kind='signing',id='unknown-flag-before-signing',key='00'*32,args=['--sign-allowlist','--unknown']),
        dict(kind='signing',id='invalid-utf8-source',key='00'*32,bytesHex='ff'),
    ])
    return result


def exercise(module,case,temporary):
    target=temporary/'linter.py'
    if case.get('directory'):target.mkdir()
    elif not case.get('missing'):target.write_bytes(bytes.fromhex(case['bytesHex']) if 'bytesHex' in case else case.get('before','ALLOWLIST_SIGNATURE: str = "old"\n').encode('utf-8'))
    env=fixture_environment(case)
    if 'RERAIL_TAPE_CACHE' in os.environ:env['RERAIL_TAPE_CACHE']=os.environ['RERAIL_TAPE_CACHE']
    out,err=io.StringIO(),io.StringIO()
    with ExitStack() as stack:
        stack.enter_context(patch.dict(os.environ,env,clear=True))
        stack.enter_context(patch.object(module,'__file__',str(target)));stack.enter_context(patch.object(module,'ALLOWLISTED_FILES',case.get('entries',[])))
        stack.enter_context(patch.object(sys,'argv',['lint_dynamic_module_loading.py',*case.get('args',['--sign-allowlist'])]))
        with redirect_stdout(out),redirect_stderr(err):
            try:value=dict(statuses=[module.cli_main() for _ in range(case.get('repeat',1))])
            except BaseException as error:value=dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=canonical(error.args,temporary),filename=getattr(error,'filename',None)))
    value.update(stdout=out.getvalue(),stderr=err.getvalue(),fileBytesBase64=base64.b64encode(target.read_bytes()).decode('ascii') if target.is_file() else None)
    return normalize(value,temporary)


if __name__=='__main__':
    from contract_runner import main
    from dynamic_port import install
    main(CHECK,'rerail.dynamic-signing-contract/v1',cases,exercise,install,['dynamic_signing_contract.py','dynamic_port.py','dynamic_cli_port.py','dispatch_signing_contract.py','impl_direct_execution_signing_contract.py'],guard_modules=['scripts.lint_lib.discovery','scripts.lint_lib.signing'])
