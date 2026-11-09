"""Compare signing, streams and complete file bytes using public synthetic key fixtures."""
import argparse
import base64
import contextlib
import importlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
from unittest.mock import patch

from migration import ROOT,PIN,compare,git,read_json,sha,write_json
from policy_routes import policy_filename
from replay import execute_native

SEED='00'*32
RFC_SEED='9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60'
BEFORE='ALLOWLIST_SIGNATURE: str = "old"  # keep this comment\n'
CASES=[dict(id='no-key',key=None),dict(id='empty-key',key=''),dict(id='short-key',key='00'),
    dict(id='odd-key',key='0'),dict(id='invalid-key',key='0g'),dict(id='late-invalid-key',key=SEED+'x'),
    dict(id='nibble-whitespace',key='0 0'),dict(id='unicode-whitespace',key='00\u00a000'),dict(id='unicode-key',key='00💥'),
    dict(id='space-only-key',key=' \t\r\n'),dict(id='long-key',key=SEED+'00'),
    dict(id='valid-zero-seed',key=SEED),dict(id='valid-one-seed',key='01'*32),dict(id='valid-rfc-seed',key=RFC_SEED),
    dict(id='uppercase-seed',key=RFC_SEED.upper()),dict(id='ascii-whitespace-between-bytes',key=' \t'+'00 \n'*32),
    *[dict(id=name,key=SEED,before=text) for name,text in [
        ('plain-assignment','ALLOWLIST_SIGNATURE = "old"\n'),
        ('typed-sentinel','ALLOWLIST_SIGNATURE: str = NEEDS_SIGNING\n'),
        ('unsigned-prefix','ALLOWLIST_SIGNATURE = UNSIGNED_SUFFIX\n'),
        ('multiple-assignments','ALLOWLIST_SIGNATURE = "a"\nALLOWLIST_SIGNATURE: str = UNSIGNED\n'),
        ('cross-line-prefix','ALLOWLIST_SIGNATURE\n :\n str\n =\n NEEDS_SIGNING # tail\n'),
        ('unicode-whitespace-prefix','ALLOWLIST_SIGNATURE\u001c:\u00a0str\u001f= "old"\n'),
        ('windows-newlines','prefix = 1\r\nALLOWLIST_SIGNATURE = "old"\r\n'),
        ('no-trailing-newline','ALLOWLIST_SIGNATURE = "old"'),
        ('single-quoted-is-unmatched',"ALLOWLIST_SIGNATURE = 'old'\n"),
        ('wrong-type-is-unmatched','ALLOWLIST_SIGNATURE: object = "old"\n'),
        ('indented-is-unmatched',' ALLOWLIST_SIGNATURE = "old"\n'),
        ('other-name-is-unmatched','ALLOWLIST_SIGNATURE_EXTRA = "old"\n'),
        ('unicode-source','# snowman ☃\nALLOWLIST_SIGNATURE = "old" # 😀\n'),
    ]],
    dict(id='unicode-allowlists',key=SEED,canBatch=['Z','é','😀','a','a'],runtime=['Ω','a']),
    dict(id='signing-precedes-check-and-legacy',key=SEED,args=['--sign-allowlist','--check','--scan-legacy-postops','--legacy-postop-ratchet-max','-1']),
    dict(id='unknown-flag-precedes-signing',key=SEED,args=['--sign-allowlist','--unknown']),
    dict(id='missing-source-file',key=SEED,missing=True),dict(id='directory-source',key=SEED,directory=True),
    dict(id='read-only-source',key=SEED,readOnly=True),
    dict(id='repeat-identical-signing',key=SEED,repeat=2),
    dict(id='missing-key-before-missing-file',key=None,missing=True),
    dict(id='invalid-key-before-missing-file',key='0g',missing=True),
]


def fixture_environment(case):
    # Whitelist ordinary process settings. Never read or copy the ambient signing key.
    env={name:os.environ[name] for name in ['PATH','HOME','LANG','TMPDIR'] if name in os.environ}
    env.update(PYTHONHASHSEED='0',PYTHONDONTWRITEBYTECODE='1')
    if case['key'] is not None:env['NERA_SIGNING_KEY']=case['key']
    return env


def normalize(value,root):
    if isinstance(value,str):return value.replace(str(root),'$ROOT')
    if isinstance(value,list):return [normalize(v,root) for v in value]
    if isinstance(value,dict):return {k:normalize(v,root) for k,v in value.items()}
    return value


def exercise(module,case,root):
    target=root/'linter.py';module.__file__=str(target)
    if case.get('directory'):target.mkdir()
    elif not case.get('missing'):
        target.write_text(case.get('before',BEFORE),encoding='utf-8')
        if case.get('readOnly'):target.chmod(0o444)
    module.CAN_BATCH_FALSE_ALLOWLIST=frozenset(case.get('canBatch',[]))
    module.LIFECYCLE_RUNTIME_STATE_ALLOWLIST=frozenset(case.get('runtime',[]))
    out,err=io.StringIO(),io.StringIO()
    with patch.object(sys,'argv',['lint_serverplan_steps.py',*case.get('args',['--sign-allowlist'])]),contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
        try:
            statuses=[module.cli_main() for _ in range(case.get('repeat',1))]
            result=dict(status=statuses[-1])
            if len(statuses)>1:result['statuses']=statuses
        except BaseException as error:
            result=dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=list(error.args),filename=getattr(error,'filename',None)))
    after=base64.b64encode(target.read_bytes()).decode('ascii') if target.is_file() else None
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue(),fileBytesBase64=after),root)


def source_one(source,case):
    sys.path[:0]=[str(source),str(source/'scripts')]
    module=importlib.import_module('scripts.lint_serverplan_steps')
    with tempfile.TemporaryDirectory(prefix='steps-sign-source-') as temporary:return exercise(module,case,Path(temporary))


def native_one(engine,case):
    from serverplan_steps_contract import FUNCTIONS
    from serverplan_steps_port import install
    from replay import sign_native
    class Violation:
        def __init__(self,*args):pass
        def __str__(self):return ''
    module=types.SimpleNamespace(Violation=Violation,ROOT=Path('/unused'),CAN_BATCH_FALSE_ALLOWLIST=frozenset(),
        LIFECYCLE_RUNTIME_STATE_ALLOWLIST=frozenset(),LEGACY_SIDE_EFFECT_TAG_ALLOWLIST=frozenset(),
        ALLOWLIST_SIGNATURE='unused',POST_OP_FATE_OVERRIDES={})
    for name in FUNCTIONS:setattr(module,name,lambda *a,**k:(_ for _ in ()).throw(AssertionError('Original semantic entry invoked')))
    class Backend:
        def __init__(self):self.originals=[];self.calls=[];self.signatures=[]
        def replace(self,owner,name,value):self.originals.append((owner,name,getattr(owner,name)));setattr(owner,name,value)
        def native(self,check,entry,files,options=None):
            result=execute_native(engine,ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/policy_filename(check,entry),files,dict(entryPoint=entry,**(options or {})))
            self.calls.append(dict(result,entry=entry));return result['diagnostics']
        def sign_ed25519(self,requests):
            result=sign_native(engine,requests,environment=fixture_environment(case))
            self.signatures.append(dict(requests=requests,replies=result));return result
    backend=Backend();install(backend,module)
    try:
        with tempfile.TemporaryDirectory(prefix='steps-sign-native-') as temporary:actual=exercise(module,case,Path(temporary))
    finally:
        for owner,name,value in reversed(backend.originals):setattr(owner,name,value)
    return actual,backend.calls,backend.signatures


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native','source-one'])
    for name in ['source','engine','baseline','out','case']:parser.add_argument('--'+name,type=Path)
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    if args.command=='source-one':
        write_json(args.out,source_one(args.source.absolute(),read_json(args.case)));return
    assert args.source and args.out and 0<=args.part<args.parts
    args.source=args.source.absolute();assert git(args.source,'rev-parse','HEAD')==PIN
    source_names=['scripts/lint_serverplan_steps.py','scripts/lint_lib/signing.py']
    for name in source_names:compare((args.source/name).read_bytes(),subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name]))
    report=dict(format='rerail.serverplan-steps-signing/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,cases=[],
        fixtureKeys='Public, synthetic seeds only. Child environments omit all ambient signing keys.',
        selection=dict(part=args.part,parts=args.parts,total=len(CASES)),sourceFiles={n:sha((args.source/n).read_bytes()) for n in source_names},
        harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in [Path(__file__),*[Path(__file__).with_name(n) for n in ['serverplan_steps_port.py','serverplan_steps_contract.py','suite.py','replay.py','policy_routes.py','migration.py']]]})
    if args.command=='native':
        args.engine=args.engine.absolute();baseline=read_json(args.baseline);assert baseline['passed'];compare(CASES,[v['input'] for v in baseline['cases']]);report['engineSha256']=sha(args.engine.read_bytes())
    for index in range(args.part,len(CASES),args.parts):
        case=CASES[index];row=dict(input=case)
        if args.command=='source':
            with tempfile.TemporaryDirectory(prefix='steps-sign-oracle-') as temporary:
                root=Path(temporary);write_json(root/'case.json',case)
                run=subprocess.run([sys.executable,str(Path(__file__).absolute()),'source-one','--source',str(args.source),'--case',str(root/'case.json'),'--out',str(root/'result.json')],env=fixture_environment(case),capture_output=True,text=True,timeout=30)
                if run.returncode:raise RuntimeError(run.stderr)
                row['expected']=read_json(root/'result.json')
        else:
            actual,calls,signatures=native_one(args.engine,case);row.update(expected=baseline['cases'][index]['expected'],actual=actual,nativeCalls=calls,signingCalls=signatures)
        report['cases'].append(row);write_json(args.out,report)
        if args.command=='native':compare(row['expected'],row['actual'])
    for name,digest in report['harnessFiles'].items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;write_json(args.out,report);print(dict(cases=len(report['cases']),passed=True))


if __name__=='__main__':main()
