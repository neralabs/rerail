"""Record full dynamic-loading CLI streams, callback order, and native scans."""
from contextlib import ExitStack,redirect_stdout,redirect_stderr
import io
from pathlib import Path
import sys
from unittest.mock import patch
from dto import canonical
from replay import write_fixture

CHECK='dynamic_module_loading'


def cases(_module):
    rows=[]
    hit=['$ROOT/api/a.py',2,'import_module("x")','a supplied description']
    def add(name,args=(),**kw):rows.append(dict(kind='cli',id=name,args=list(args),**kw))
    for check in [False,True]:
        args=['--check'] if check else []
        add('clean-'+str(check),args,findings=[])
        add('positive-'+str(check),args,findings=[hit])
        add('duplicates-'+str(check),args,findings=[hit,hit,dict_to_hit(hit,path='$ROOT/z.py',line=-2)])
        add('verify-fails-'+str(check),args,verifyResult=False)
        add('scan-error-'+str(check),args,scanError='scanner failed')
        add('resolver-error-'+str(check),args,resolveError='resolver failed')
    for value in [False,True,'truthy']:
        add('sign-'+str(value),['--sign-allowlist'],signResult=value)
        add('sign-before-check-'+str(value),['--check','--sign-allowlist'],signResult=value,verifyResult=False)
    for args in [['--help'],['-h'],['--unknown'],['--check','unexpected'],['--root']]:add('parse-'+str(len(rows)),args)
    for length in [0,99,100,101,1200]:
        add('clip-'+str(length),findings=[dict_to_hit(hit,content='😀'*length)])
    add('callback-order',findings=[dict_to_hit(hit,path=p,line=l,content=s) for p,l,s in [('$ROOT/z.py',4,'last'),('$ROOT/a.py',1,'first'),('$ROOT/a.py',1,'duplicate')]])
    add('callback-controls',verifyStdout='verification output\n',verifyStderr='verification error\n',resolveStdout='resolve output\n',scanStdout='scan output\n',findings=[hit])
    add('verify-fails-streams',verifyResult=False,verifyStdout='verify output\n',verifyStderr='verify error\n')
    add('sign-streams',['--sign-allowlist'],signResult=True,signStdout='signed output\n',signStderr='signed error\n')
    add('allowlist-order',['--sign-allowlist'],allowed=['z.py','a.py','a.py','é.py'],signResult=True)
    add('verify-allowlist-order',allowed=['z.py','a.py','a.py','é.py'],findings=[])
    add('outside-first',findings=[dict_to_hit(hit,path='/outside.py')])
    add('outside-after-positive',findings=[hit,dict_to_hit(hit,path='/outside.py')])
    add('real-positive',files=[dict(path='api/a.py',source='importlib.import_module("os")\n')])
    add('real-positive-check',['--check'],files=[dict(path='api/a.py',source='__import__("os")\n')])
    for args in [['--root','$ROOT'],['--root','$ROOT','--dirs'],['--root','$ROOT','--dirs','public','impl','public'],['--root','$ROOT','--dirs','impl','impl']]:
        add('real-resolver-'+str(len(rows)),args,resolveReal=True,files=[dict(path='impl/a.py',source='__import__("os")\n'),dict(path='api/b.py',source='import_module("x")\n')])
    for signature in ['', 'UNSIGNED','NEEDS_SIGNING','!!!!','a','a===']:
        add('native-verification-'+str(len(rows)),signature=signature,verifyNative=True,findings=[])
    add('native-verification-valid',verifyNative=True,findings=[])
    return rows


def dict_to_hit(hit,**kw):
    names=['path','line','content','description'];return [kw.get(k,v) for k,v in zip(names,hit)]


def exercise(module,case,temporary):
    root=temporary/'repo';root.mkdir();write_fixture(root,case.get('files',[]));events=[]
    real_scan=module.find_violations;real_resolve=module.resolve_root_and_dirs
    def canon(value):
        # macOS resolves /var to /private/var. Normalize only this fixture's aliases.
        aliases=[temporary,temporary.resolve()]
        def align(v):
            if isinstance(v,Path):
                for alias in aliases:
                    if v.is_absolute() and v.is_relative_to(alias):return temporary/v.relative_to(alias)
                return v
            if isinstance(v,str):
                for alias in aliases:
                    prefix=str(alias)+'/'
                    if v.startswith(prefix):return str(temporary)+'/'+v[len(prefix):]
                for alias in aliases:v=v.replace(str(alias),'$TEMP')
                return v
            if isinstance(v,tuple):return tuple(align(x) for x in v)
            if isinstance(v,list):return [align(x) for x in v]
            if isinstance(v,dict):return {k:align(x) for k,x in v.items()}
            return v
        return canonical(align(value),temporary)
    def streams(prefix):
        sys.stdout.write(case.get(prefix+'Stdout',''));sys.stderr.write(case.get(prefix+'Stderr',''))
    def verify(*args,**kw):events.append(dict(name='verify',args=canon(args),kwargs=canon(kw)));streams('verify');return case.get('verifyResult',True)
    def sign(*args,**kw):events.append(dict(name='sign',args=canon(args),kwargs=canon(kw)));streams('sign');return case.get('signResult',False)
    def resolve(args,**kw):
        events.append(dict(name='resolve',args=canon(vars(args)),kwargs=canon(kw)));streams('resolve')
        if 'resolveError' in case:raise RuntimeError(case['resolveError'])
        return real_resolve(args,**kw) if case.get('resolveReal') else (root,case.get('directories',['api']))
    def scan(actual_root,dirs):
        events.append(dict(name='scan',args=canon((actual_root,dirs))));streams('scan')
        if 'scanError' in case:raise RuntimeError(case['scanError'])
        if 'findings' in case:return [(Path(p.replace('$ROOT',str(root))),line,text,desc) for p,line,text,desc in case['findings']]
        return real_scan(actual_root,dirs)
    out,err=io.StringIO(),io.StringIO()
    with ExitStack() as stack:
        stack.enter_context(patch.object(sys,'argv',['lint_dynamic_module_loading.py',*[arg.replace('$ROOT',str(root)) for arg in case['args']]]))
        stack.enter_context(patch.object(module,'find_violations',scan));stack.enter_context(patch.object(module,'resolve_root_and_dirs',resolve));stack.enter_context(patch.object(module,'sign_and_update_allowlist',sign))
        if not case.get('verifyNative'):stack.enter_context(patch.object(module,'verify_allowlist',verify))
        if 'allowed' in case:stack.enter_context(patch.object(module,'ALLOWLISTED_FILES',case['allowed']))
        if 'signature' in case:stack.enter_context(patch.object(module,'ALLOWLIST_SIGNATURE',case['signature']))
        with redirect_stdout(out),redirect_stderr(err):
            try:value=dict(status=module.cli_main())
            except BaseException as error:value=dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=canon(error.args)))
    return dict(value,stdout=out.getvalue().replace(str(root),'$ROOT').replace(str(root.resolve()),'$ROOT'),stderr=err.getvalue().replace(str(root),'$ROOT').replace(str(root.resolve()),'$ROOT'),events=events)


if __name__=='__main__':
    from contract_runner import main
    from dynamic_port import install
    main(CHECK,'rerail.dynamic-cli-contract/v1',cases,exercise,install,['dynamic_cli_contract.py','dynamic_port.py','dynamic_cli_port.py'],guard_modules=['scripts.lint_lib.discovery','scripts.lint_lib.signing'])
