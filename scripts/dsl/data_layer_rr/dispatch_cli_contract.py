"""Record complete dispatch CLI streams, decisions, I/O errors, and callbacks."""
import argparse
from contextlib import ExitStack,redirect_stdout,redirect_stderr
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from unittest.mock import patch
from dto import canonical
from migration import ROOT,PIN,compare,git,read_json,write_json,sha
from replay import write_fixture

NAME='scripts.lint_serverplan_dispatch_architecture'
INVENTORY='scripts/allowlists/lint_serverplan_dispatch_architecture.txt'


def cases():
    hit=dict(code='CUSTOM',path='impl/a.py',line=4,scope='C.f',expression='a  + b',message='guidance',occurrence=1)
    rows=[]
    def add(name,args=(),files=(),**extra):rows.append(dict(id=name,args=list(args),files=list(files),**extra))
    for check in [False,True]:
        args=['--check'] if check else []
        add('clean-'+str(check),args)
        add('callback-positive-'+str(check),args,findings=[hit])
        add('callback-duplicates-'+str(check),args,findings=[hit,hit,dict(hit,code='AAA',line=-1)])
        add('callback-verify-false-'+str(check),args,findings=[],verifyResult=False)
        add('invalid-signature-'+str(check),args,signature='!!!!')
        add('scan-error-'+str(check),args,scanError='caller scan failure')
    for args in [['--sign-inventory'],['--sign-allowlist'],['--check','--sign-inventory']]:
        for value in [False,True]:add('sign-'+str(len(rows)),args,signResult=value)
    add('help',['--help']);add('bad-flag',['--unknown']);add('bad-value',['--check','yes'])
    for name,text in [('empty',''),('comment',' # comment\n \n'),('entries','entry\nentry\n'),
        ('unicode','\u2000# comment\u0085a\u2028b\u2029'),('nul','a\0b\n')]:
        for args in [[],['--check'],['--sign-inventory']]:
            add('inventory-'+name+'-'+str(len(rows)),args,[dict(path=INVENTORY,source=text)],signResult=True)
    add('inventory-directory',files=[dict(path=INVENTORY,directory=True)])
    add('inventory-invalid-utf8',files=[dict(path=INVENTORY,bytesHex='6162ff6364')])
    add('inventory-dangling-link',files=[dict(path=INVENTORY,symlink='missing.txt')])
    add('inventory-file-link',files=[dict(path='shared.txt',source='entry\n'),dict(path=INVENTORY,symlink='../../shared.txt')])
    add('real-positive',files=[dict(path='impl/common/l2065_plan_core/a.py',source='ctx.is_sql_backend\n')])
    add('real-positive-check',['--check'],[dict(path='impl/common/l2065_plan_core/a.py',source='ctx.is_sql_backend\n')])
    add('real-base',['--check'],[dict(path='impl/common/l2070_server_plan/base_server_plan.py',source='def f(backend):\n backend.transport\n')])
    add('callback-paths',findings=[dict(hit,path=p) for p in ['a.py','$ROOT/inside.py','/outside.py','//network/a.py','a.py']])
    add('callback-order',findings=[dict(hit,code=c,path=p,expression=e) for c,p,e in [('Z','z.py','z'),('A','a.py','a\nb'),('Z','a.py','☃')]])
    add('verify-streams',['--check'],findings=[hit],verifyResult=False,verifyStdout='verify output\n',verifyStderr='verify error\n')
    add('sign-streams',['--sign-inventory'],signResult=True,signStdout='sign output\n',signStderr='sign error\n')
    add('nonempty-stops-callbacks',['--check','--sign-inventory'],[dict(path=INVENTORY,source='entry\n')],scanError='must not scan',signResult=True,verifyResult=False)
    return rows


def fixture(root,files):
    regular=[f for f in files if 'bytesHex' not in f];write_fixture(root,regular)
    for f in files:
        if 'bytesHex' in f:
            p=root/f['path'];p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(bytes.fromhex(f['bytesHex']))


def load_source(source,root):
    original=source/'scripts/lint_serverplan_dispatch_architecture.py'
    path=root/'scripts/lint_serverplan_dispatch_architecture.py';path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(original.read_bytes())
    spec=importlib.util.spec_from_file_location(NAME,path);module=importlib.util.module_from_spec(spec)
    return module,spec


def exercise(module,case,root):
    calls=[]
    def scan():
        calls.append(dict(name='scan'))
        if 'scanError' in case:raise RuntimeError(case['scanError'])
        if 'findings' in case:
            return tuple(module.Finding(**dict(v,path=Path(v['path'].replace('$ROOT',str(root))))) for v in case['findings'])
        return real_scan()
    def verify(*args):
        calls.append(dict(name='verify',args=canonical(args,root)))
        sys.stdout.write(case.get('verifyStdout',''));sys.stderr.write(case.get('verifyStderr',''))
        return case['verifyResult']
    def sign(*args):
        calls.append(dict(name='sign',args=canonical(args,root)))
        sys.stdout.write(case.get('signStdout',''));sys.stderr.write(case.get('signStderr',''))
        return case.get('signResult',False)
    real_scan=module.scan_repository;out,err=io.StringIO(),io.StringIO()
    with ExitStack() as stack:
        stack.enter_context(patch.object(module,'scan_repository',scan))
        if 'verifyResult' in case:stack.enter_context(patch.object(module,'verify_allowlist',verify))
        if 'signature' in case:stack.enter_context(patch.object(module,'INVENTORY_SIGNATURE',case['signature']))
        stack.enter_context(patch.object(module,'sign_and_update_allowlist',sign))
        stack.enter_context(patch.object(sys,'argv',['lint_serverplan_dispatch_architecture.py',*case['args']]))
        with redirect_stdout(out),redirect_stderr(err):
            try:value=dict(status=module.cli_main())
            except BaseException as error:
                value=dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=canonical(error.args,root)))
                if isinstance(error,OSError):value['error'].update(errno=error.errno,filename=str(error.filename).replace(str(root),'$ROOT'))
    return dict(value,stdout=out.getvalue().replace(str(root),'$ROOT'),stderr=err.getvalue().replace(str(root),'$ROOT'),calls=calls)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path,required=n in ['source','out'])
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args()
    if not 0<=a.part<a.parts:p.error('Invalid shard')
    source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    path=source/'scripts/lint_serverplan_dispatch_architecture.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':scripts/lint_serverplan_dispatch_architecture.py']))
    sys.path[:0]=[str(source),str(source/'scripts')];rows=[];fallbacks=[]
    if a.command=='native':
        baseline=read_json(a.baseline);compare(baseline['passed'],True);compare(baseline['sourcePin'],PIN)
        compare(baseline['sourceSha256'],sha(path.read_bytes()));compare([r['input'] for r in baseline['cases']],cases())
    files=[Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','replay.py','dto.py']]]
    if a.command=='native':files.extend(Path(__file__).with_name(n) for n in ['dispatch_cli_port.py','dispatch_cli_workspace.py'])
    selected=sum(a.command=='source' or i%a.parts==a.part for i in range(len(cases())))
    report=dict(format='rerail.dispatch-cli/v1',sourcePin=PIN,sourceSha256=sha(path.read_bytes()),admitted=False,oracle=a.command=='source',
        mode=a.mode,part=a.part,parts=a.parts,cases=rows,semanticFallbacks=fallbacks,harnessFiles={f.relative_to(ROOT).as_posix():sha(f.read_bytes()) for f in files},
        expectedCases=selected,complete=False,passed=False)
    if a.command=='native':report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()))
    write_json(a.out,report)
    for i,case in enumerate(cases()):
        if a.command=='native' and i%a.parts!=a.part:continue
        started=time.monotonic()
        with tempfile.TemporaryDirectory(prefix='dispatch-cli-') as temporary:
            root=Path(temporary).resolve();fixture(root,case['files']);module,spec=load_source(source,root)
            with patch.dict(sys.modules,{NAME:module}):
                spec.loader.exec_module(module)
                if a.command=='native':
                    from dispatch_cli_port import install
                    from data_layer_audit.native_guard import codes
                    original=codes({'serverplan_dispatch_architecture':module})
                    executions=[];install(a.engine.absolute(),module,a.mode,executions)
                    def guard(frame,event,_arg):
                        if event=='call' and frame.f_code in original and not original[frame.f_code][2]:
                            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
                    sys.setprofile(guard)
                try:value=exercise(module,case,root)
                finally:sys.setprofile(None)
        row=dict(input=case,seconds=round(time.monotonic()-started,3))
        if a.command=='source':row['expected']=value
        else:row.update(expected=baseline['cases'][i]['expected'],actual=value,native=executions,passed=value==baseline['cases'][i]['expected'])
        rows.append(row)
        write_json(a.out,report)
        print(dict(id=case['id'],seconds=row['seconds'],passed=row.get('passed',True)),flush=True)
    if a.command=='source':
        by_id={r['input']['id']:r['expected'] for r in rows}
        compare(by_id['real-positive']['status'],1);compare('SPD001' in by_id['real-positive']['stdout'],True)
        compare(by_id['real-base']['status'],1);compare('SPD005' in by_id['real-base']['stdout'],True)
        compare(by_id['invalid-signature-False']['status'],0);compare(by_id['invalid-signature-False']['stderr'],'')
        compare(by_id['invalid-signature-True']['status'],1);compare(bool(by_id['invalid-signature-True']['stderr']),True)
    report.update(complete=len(rows)==selected,passed=len(rows)==selected and all(r.get('passed',True) for r in rows) and not fallbacks)
    write_json(a.out,report);print(dict(cases=len(rows),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
