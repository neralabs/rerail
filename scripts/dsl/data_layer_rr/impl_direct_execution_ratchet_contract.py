"""Record exact CLI ratchet observations, including real baseline reads and writes."""
import argparse
import base64
import contextlib
from datetime import datetime, UTC
import importlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT, PIN, compare, git, read_json, sha, write_json

NAME='lint_impl_direct_execution'
BASELINE='.ratchet-baselines/'+NAME+'.json'
PUBLIC_SIGNATURE='QKS3l/2jsmXFom/kzPqfq3JV2xu5T+mL3cHNvbaZGsaVaqjPYhgU63EmmCpvgeadKedTixg2ZzLa7TVIE5ZyCg=='
NOW='2026-11-10T12:34:56.123456+00:00'

def baseline(n=0,signature='UNSIGNED'):
    return dict(counts=dict(violations=n),_metadata=dict(signature=signature))

CASES=[
    dict(id='missing-clean',count=0),dict(id='missing-positive',count=1),
    dict(id='require-baseline-flag-is-not-forwarded',count=1,args=['--check','--require-baseline']),
    dict(id='unsigned-required',count=0,baseline=baseline()),
    dict(id='unsigned-skipped-clean',count=0,baseline=baseline(),skip=True),
    dict(id='regression',count=2,baseline=baseline(1),skip=True),
    dict(id='improvement',count=1,baseline=baseline(3),skip=True),
    dict(id='equal',count=2,baseline=baseline(2),skip=True),
    dict(id='real-signature-clean',count=0,baseline=baseline(0,PUBLIC_SIGNATURE)),
    dict(id='real-signature-regression',count=1,baseline=baseline(0,PUBLIC_SIGNATURE)),
    dict(id='tampered-count',count=1,baseline=baseline(1,PUBLIC_SIGNATURE)),
    dict(id='skip-still-verifies-existing-signature',count=0,baseline=baseline(0,'tampered'),skip=True),
    dict(id='missing-signature',count=0,baseline=dict(counts=dict(violations=0),_metadata={})),
    dict(id='metadata-not-object',count=1,baseline=dict(counts=dict(violations=1),_metadata=[])),
    dict(id='metadata-null',count=0,baseline=dict(counts=dict(violations=0),_metadata=None)),
    dict(id='counts-not-object',count=1,baseline=dict(counts=[],_metadata=dict(signature='UNSIGNED'))),
    dict(id='empty-counts',count=1,baseline=dict(counts={},_metadata=dict(signature='UNSIGNED')),skip=True),
    dict(id='float-count',count=1,baseline=baseline(1.9),skip=True),
    dict(id='string-count-not-a-comparison-ceiling',count=1,baseline=baseline('2'),skip=True),
    dict(id='boolean-count',count=1,baseline=baseline(True),skip=True),
    dict(id='array-root',count=0,baseline=[]),
    dict(id='malformed-json',count=1,raw='{'),
    dict(id='utf8-bom',count=0,raw='\ufeff{}'),
    dict(id='invalid-utf8',count=0,rawBytes='/w=='),
    dict(id='ignored-without-check',count=1,raw='{',args=[]),
    dict(id='update-clean',count=0,args=['--update-baseline']),
    dict(id='update-positive',count=2,args=['--update-baseline','--reason','fixture ☃']),
    dict(id='update-precedes-check',count=1,args=['--check','--update-baseline'],raw='{'),
    dict(id='update-sign-missing-key',count=1,args=['--update-baseline','--sign']),
    dict(id='update-sign-public-fixture',count=1,args=['--update-baseline','--sign','--reason','public synthetic seed'],key='00'*32),
]

class FixedDatetime:
    @staticmethod
    def now(tz):
        assert tz is UTC
        return datetime.fromisoformat(NOW)


def environment(case):
    # Never copy an ambient signing key into fixtures.
    env={k:os.environ[k] for k in ['PATH','HOME','LANG','TMPDIR'] if k in os.environ}
    env.update(PYTHONHASHSEED='0',PYTHONDONTWRITEBYTECODE='1',GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_COUNT='1',GIT_CONFIG_KEY_0='user.email',GIT_CONFIG_VALUE_0='fixture@example.invalid')
    if 'key' in case:env['NERA_SIGNING_KEY']=case['key']
    return env


def normalize(value,root):
    if isinstance(value,str):return value.replace(str(root),'$ROOT')
    if isinstance(value,list):return [normalize(v,root) for v in value]
    if isinstance(value,tuple):return [normalize(v,root) for v in value]
    if isinstance(value,dict):return {k:normalize(v,root) for k,v in value.items()}
    return value


def exercise(module,case,root):
    (root/'impl').mkdir()
    if case['count']:(root/'impl/offender.py').write_text('obj._fetchone()\n'*case['count'],encoding='utf-8')
    target=root/BASELINE
    if any(k in case for k in ['baseline','raw','rawBytes']):
        target.parent.mkdir(parents=True)
        raw=base64.b64decode(case['rawBytes']) if 'rawBytes' in case else case.get('raw',json.dumps(case.get('baseline'))).encode('utf-8')
        target.write_bytes(raw)
    before=base64.b64encode(target.read_bytes()).decode() if target.exists() else None
    argv=case.get('args',['--check']+(['--skip-signature-check'] if case.get('skip') else []))
    out,err=io.StringIO(),io.StringIO()
    with patch.object(module,'__file__',str(root/'scripts/linter.py')),patch.object(module.lint_ratchet,'get_repo_root',lambda:root),patch.object(module.lint_ratchet,'datetime',FixedDatetime),patch.object(sys,'argv',['lint_impl_direct_execution.py',*argv]),contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
        try:result=dict(status=module.cli_main())
        except BaseException as error:result=dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=error.args,filename=getattr(error,'filename',None)))
    after=base64.b64encode(target.read_bytes()).decode() if target.exists() else None
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue(),baselineBefore=before,baselineAfter=after),root)


def source_one(source,case):
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_impl_direct_execution')
    with tempfile.TemporaryDirectory(prefix='impl-direct-ratchet-source-') as temporary:return exercise(module,case,Path(temporary))


def native_one(engine,case,source):
    from dataclasses import make_dataclass
    from types import SimpleNamespace
    from impl_direct_execution_contract import FUNCTIONS
    from impl_direct_execution_port import install
    from replay import execute_native, sign_native
    from policy_routes import policy_filename
    class Backend:
        def __init__(self):self.calls=[];self.signatures=[]
        def replace(self,owner,name,value):setattr(owner,name,value)
        def native(self,check,entry,files,options=None):
            value=execute_native(engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/policy_filename(check,entry),files,dict(entryPoint=entry,**(options or {})))
            self.calls.append(dict(value,entry=entry));return value['diagnostics']
        def sign_ed25519(self,requests):
            value=sign_native(engine,requests,environment=environment(case));self.signatures.append(dict(requests=requests,replies=value));return value
    backend=Backend();settings=backend.native('impl_direct_execution','configuration',[],dict(env=['ide.api=true']))
    allowed={r['parameters']['value'] for r in settings if r['parameters']['setting']=='allowed_file'}
    module=SimpleNamespace(PROJECT_ROOT=source,ALLOWED_FILES=allowed,Violation=make_dataclass('Violation',[(k,t) for k,t in [('file',Path),('line_num',int),('line',str),('pattern',str)]]),
        lint_signing=SimpleNamespace(verify_allowlist=None),lint_ratchet=SimpleNamespace(check_ratchet=None,update_ratchet_baseline=None,get_repo_root=None,datetime=FixedDatetime),__file__='/unused/scripts/linter.py')
    for name in [*FUNCTIONS,'ALLOWED_DIR_PATTERNS','DIRECT_EXECUTION_PATTERNS','FTS_MANAGER_DIRECT_EXECUTION_PATTERNS','_ALLOWED_FILES_SIGNATURE']:setattr(module,name,None)
    install(backend,module)
    with tempfile.TemporaryDirectory(prefix='impl-direct-ratchet-native-') as temporary:actual=exercise(module,case,Path(temporary))
    return actual,backend.calls,backend.signatures


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','source-one','native'])
    for name in ['source','out','case','engine','baseline']:p.add_argument('--'+name,type=Path,required=name in ['source','out'])
    p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1)
    a=p.parse_args();a.source=a.source.absolute()
    if a.command=='source-one':write_json(a.out,source_one(a.source,read_json(a.case)));return
    assert git(a.source,'rev-parse','HEAD')==PIN
    paths=['scripts/lint_impl_direct_execution.py','scripts/lint_lib/ratchet.py','scripts/lint_lib/signing.py']
    for name in paths:compare((a.source/name).read_bytes(),subprocess.check_output(['git','-C',str(a.source),'show',PIN+':'+name]))
    report=dict(format='rerail.impl-direct-ratchet-contract/v1',mode=a.command,sourcePin=PIN,fullMigrationAcceptance=False,passed=False,cases=[],sourceFiles={n:sha((a.source/n).read_bytes()) for n in paths},harness=Path(__file__).read_text())
    if a.command=='native':
        if any(n.endswith('lint_impl_direct_execution') for n in sys.modules):raise ValueError('Native ratchet tests cannot import the source detector')
        expected=read_json(a.baseline);assert expected['passed'];compare(CASES,[r['input'] for r in expected['cases']]);report['engineSha256']=sha(a.engine.read_bytes())
    assert 0<=a.part<a.parts
    report['selection']=dict(part=a.part,parts=a.parts,total=len(CASES))
    for index in range(a.part,len(CASES),a.parts):
        case=CASES[index]
        if a.command=='native':
            actual,calls,signatures=native_one(a.engine.absolute(),case,a.source);report['cases'].append(dict(input=case,expected=expected['cases'][index]['expected'],actual=actual,native=calls,signing=signatures,passed=actual==expected['cases'][index]['expected']));write_json(a.out,report);continue
        with tempfile.TemporaryDirectory(prefix='impl-direct-ratchet-oracle-') as temporary:
            root=Path(temporary);write_json(root/'case.json',case)
            run=subprocess.run([sys.executable,str(Path(__file__).absolute()),'source-one','--source',str(a.source),'--case',str(root/'case.json'),'--out',str(root/'result.json')],env=environment(case),capture_output=True,text=True,timeout=30)
            if run.returncode:raise RuntimeError(run.stderr)
            report['cases'].append(dict(input=case,expected=read_json(root/'result.json')));write_json(a.out,report)
    report['passed']=all(r.get('passed',True) for r in report['cases']);write_json(a.out,report);print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)

if __name__=='__main__':main()
