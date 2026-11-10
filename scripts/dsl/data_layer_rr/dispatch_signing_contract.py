"""Compare actual dispatch signing and complete file bytes with public test seeds."""
import argparse
import base64
from contextlib import redirect_stdout,redirect_stderr
import importlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT,PIN,compare,git,read_json,sha,write_json
from impl_direct_execution_signing_contract import fixture_environment,normalize

SEED='00'*32
VARIABLE='INVENTORY_SIGNATURE'
CASES=[dict(id=name,key=value) for name,value in [
    ('missing',None),('empty',''),('short','00'),('odd','0'),('invalid','0g'),
    ('late-invalid',SEED+'x'),('nibble-space','0 0'),('unicode-space','00\u00a000'),
    ('space-only',' \t\r\n'),('long',SEED+'00'),('zero-seed',SEED),('one-seed','01'*32),
    ('whitespace-seed',' \t'+'00 \n'*32)]]
CASES += [dict(id=name,key=SEED,before=text) for name,text in [
    ('plain','INVENTORY_SIGNATURE = "old"\n'),
    ('typed','INVENTORY_SIGNATURE: str = "old" # tail\n'),
    ('sentinel','INVENTORY_SIGNATURE: str = NEEDS_SIGNING\n'),
    ('unsigned-prefix','INVENTORY_SIGNATURE = UNSIGNED_SUFFIX\n'),
    ('multiple','INVENTORY_SIGNATURE = "a"\nINVENTORY_SIGNATURE = UNSIGNED\n'),
    ('cross-line','INVENTORY_SIGNATURE\n :\n str\n =\n NEEDS_SIGNING # tail\n'),
    ('unicode-space-assignment','INVENTORY_SIGNATURE\u001c:\u00a0str\u001f= "old"\n'),
    ('windows','prefix = 1\r\nINVENTORY_SIGNATURE = "old"\r\n'),
    ('no-newline','INVENTORY_SIGNATURE = "old"'),
    ('single-quoted',"INVENTORY_SIGNATURE = 'old'\n"),
    ('wrong-type','INVENTORY_SIGNATURE: object = "old"\n'),
    ('indented',' INVENTORY_SIGNATURE = "old"\n'),
    ('wrong-name','INVENTORY_SIGNATURE_EXTRA = "old"\n'),
    ('unicode-source','# snowman ☃\nINVENTORY_SIGNATURE = "old" # 😀\n')]]
CASES += [dict(id='missing-file',key=SEED,missing=True),dict(id='directory-file',key=SEED,directory=True),
    dict(id='repeat',key=SEED,repeat=2),dict(id='key-error-before-file-error',key=None,missing=True)]


def exercise(module,case,root):
    target=root/'linter.py'
    if case.get('directory'):target.mkdir()
    elif not case.get('missing'):target.write_bytes(case.get('before','INVENTORY_SIGNATURE: str = "old"\n').encode('utf-8'))
    out,err=io.StringIO(),io.StringIO()
    with redirect_stdout(out),redirect_stderr(err):
        try:result=dict(statuses=[module.sign_and_update_allowlist(target,'serverplan_dispatch_architecture',[],VARIABLE) for _ in range(case.get('repeat',1))])
        except BaseException as error:result=dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=list(error.args),filename=getattr(error,'filename',None)))
    result.update(stdout=out.getvalue(),stderr=err.getvalue(),fileBytesBase64=base64.b64encode(target.read_bytes()).decode('ascii') if target.is_file() else None)
    return normalize(result,root)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','source-one','native'])
    for n in ['source','engine','baseline','out','case']:p.add_argument('--'+n,type=Path)
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args()
    source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    if a.command=='source-one':
        with tempfile.TemporaryDirectory(prefix='dispatch-sign-oracle-') as temporary:write_json(a.out,exercise(module,read_json(a.case),Path(temporary)))
        return
    if not 0<=a.part<a.parts:p.error('Invalid shard')
    names=['scripts/lint_serverplan_dispatch_architecture.py','scripts/lint_lib/signing.py']
    for name in names:compare((source/name).read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+name]))
    rows=[];fallbacks=[];baseline=read_json(a.baseline) if a.command=='native' else None
    harness=[Path(__file__),*[Path(__file__).with_name(n) for n in ['dispatch_cli_port.py','impl_direct_execution_signing_contract.py','migration.py','replay.py']]]
    report=dict(format='rerail.dispatch-signing/v1',sourcePin=PIN,admitted=False,passed=False,complete=False,mode=a.mode,part=a.part,parts=a.parts,cases=rows,
        semanticFallbacks=fallbacks,fixtureKeys='Public synthetic seeds only; child environments omit ambient signing keys.',
        sourceFiles={n:sha((source/n).read_bytes()) for n in names},harnessFiles={f.relative_to(ROOT).as_posix():sha(f.read_bytes()) for f in harness})
    if baseline:
        compare(baseline['passed'],True);compare(baseline['sourceFiles'],report['sourceFiles']);compare([r['input'] for r in baseline['cases']],CASES)
        report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()))
    write_json(a.out,report)
    if a.command=='native':
        from data_layer_audit.native_guard import codes
        signing=importlib.import_module('scripts.lint_lib.signing')
        original=codes({'dispatch':module,'signing':signing})
        def guard(frame,event,_arg):
            if event=='call' and frame.f_code in original and not original[frame.f_code][2]:
                fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
        from dispatch_cli_port import install
        executions=[];install(a.engine.absolute(),module,a.mode,executions)
    for i,case in enumerate(CASES):
        if a.command=='native' and i%a.parts!=a.part:continue
        row=dict(input=case)
        if a.command=='source':
            with tempfile.TemporaryDirectory(prefix='dispatch-sign-source-') as temporary:
                root=Path(temporary);write_json(root/'case.json',case)
                run=subprocess.run([sys.executable,str(Path(__file__).absolute()),'source-one','--source',str(source),'--case',str(root/'case.json'),'--out',str(root/'out.json')],capture_output=True,text=True,timeout=30,env=fixture_environment(case))
                if run.returncode:raise RuntimeError(run.stderr)
                row['expected']=read_json(root/'out.json')
        else:
            executions.clear()
            with tempfile.TemporaryDirectory(prefix='dispatch-sign-native-') as temporary,patch.dict('os.environ',fixture_environment(case),clear=True):
                sys.setprofile(guard)
                try:actual=exercise(module,case,Path(temporary))
                finally:sys.setprofile(None)
            row.update(expected=baseline['cases'][i]['expected'],actual=actual,native=list(executions),passed=actual==baseline['cases'][i]['expected'])
        rows.append(row);write_json(a.out,report);print(dict(id=case['id'],passed=row.get('passed',True)),flush=True)
    report.update(complete=True,passed=all(r.get('passed',True) for r in rows) and not fallbacks)
    write_json(a.out,report);raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
