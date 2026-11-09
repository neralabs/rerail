"""Compare exact CLI streams, statuses and positive findings on physical fixtures."""
import argparse
import contextlib
import importlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
from unittest.mock import patch

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_native, write_fixture
from policy_routes import policy_filename

BAD = '''class BadStep(ServerPlanStep):
    def preprocess(self, resolved): self._value = resolved.get('x')
    def compile(self, ctx):
        if self._value: return 1
        return 2
'''
LEGACY = '''def work(raw, hydrator):
    hydrator(raw)
    values.append(raw)
    return self._transform(raw)
x = CompiledStep(post_op=work, post_op_with_refs=work)
'''
CASES = [
    dict(id='clean',files=[dict(path='steps/a.py',source='pass\n')]),
    *[dict(id='positive-'+str(check),args=['--check'] if check else [],files=[dict(path='steps/b.py',source=BAD)]) for check in [False,True]],
    dict(id='verbose-order-and-duplicate-scans',args=['--verbose','--check'],dirs=['steps','steps/sub','steps'],extras=['steps/a.py','other/b.py','missing.py'],files=[dict(path=p,source=BAD) for p in ['steps/a.py','steps/a/inner.py','steps/sub/c.py','steps/tests/d.py','steps/.hidden/e.py','other/b.py']]),
    dict(id='links',args=['--verbose'],files=[dict(path='steps/a.py',source=BAD),dict(path='steps/alias.py',symlink='a.py'),dict(path='steps/sub/a.py',source=BAD),dict(path='steps/link',symlink='sub')]),
    dict(id='relative-directory',relative=True,args=['--verbose'],files=[dict(path='steps/a.py',source=BAD)]),
    dict(id='parse-error',args=['--check'],files=[dict(path='steps/a.py',source='def :\n')]),
    dict(id='null-byte',files=[dict(path='steps/a.py',source='x = "\x00"\n')]),
    dict(id='empty-missing',dirs=['missing'],files=[]),
    *[dict(id='legacy-'+str(limit),args=['--scan-legacy-postops']+([] if limit is None else ['--legacy-postop-ratchet-max',str(limit)]),files=[dict(path='steps/a.py',source=LEGACY)]) for limit in [None,-1,0,6,99]],
    dict(id='legacy-empty',args=['--scan-legacy-postops'],files=[]),
    dict(id='signature-tampered',signature='tampered',args=['--check'],files=[dict(path='steps/a.py',source=BAD)]),
    dict(id='signature-unsigned',signature='NEEDS_SIGNING',files=[]),
    dict(id='unknown-flag',args=['--does-not-exist'],files=[]),
    dict(id='invalid-ratchet',args=['--legacy-postop-ratchet-max','no'],files=[]),
    dict(id='help',args=['--help'],files=[]),
]


def normalize(value, root):
    return value.replace(str(root),'$ROOT') if isinstance(value,str) else value


def invoke(module, case, root):
    output,errors = io.StringIO(),io.StringIO()
    with patch.object(sys,'argv',['lint_serverplan_steps.py',*case.get('args',[])]),contextlib.redirect_stdout(output),contextlib.redirect_stderr(errors):
        try:
            status = module.cli_main();result = dict(status=status)
        except SystemExit as error:
            result = dict(error=dict(type='builtins.SystemExit',code=error.code))
    return dict(result,stdout=normalize(output.getvalue(),root),stderr=normalize(errors.getvalue(),root))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','out','baseline','engine']:parser.add_argument('--'+name,type=Path,required=name in ['source','out'])
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    if not 0 <= args.part < args.parts:parser.error('Invalid partition')
    args.source=args.source.absolute()
    if args.engine:args.engine=args.engine.absolute()
    assert git(args.source,'rev-parse','HEAD') == PIN
    filename='scripts/lint_serverplan_steps.py'
    compare((args.source/filename).read_bytes(),subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+filename]))
    report=dict(format='rerail.serverplan-steps-cli-contract/v1',sourcePin=PIN,fullMigrationAcceptance=False,
        passed=False,cases=[],selection=dict(part=args.part,parts=args.parts,total=len(CASES)),
        separateActionContracts={'--sign-allowlist':'serverplan_steps_signing_contract.py'},harnessFiles={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in
        [Path(__file__),*[Path(__file__).with_name(n) for n in ['serverplan_steps_port.py','replay.py','policy_routes.py','migration.py']]]})
    if args.command == 'source':
        sys.path.insert(0,str(args.source.absolute()));module=importlib.import_module('scripts.lint_serverplan_steps')
        config=dict(canBatchAllowlist=sorted(module.CAN_BATCH_FALSE_ALLOWLIST),runtimeStateAllowlist=sorted(module.LIFECYCLE_RUNTIME_STATE_ALLOWLIST),
            sideEffectAllowlist=sorted(module.LEGACY_SIDE_EFFECT_TAG_ALLOWLIST),signature=module.ALLOWLIST_SIGNATURE,
            fates={name:value.value for name,value in module.POST_OP_FATE_OVERRIDES.items()})
    else:
        baseline=read_json(args.baseline);assert baseline['passed'];compare(CASES,[r['input'] for r in baseline['cases']]);config=baseline['config']
        report['engineSha256']=sha(args.engine.read_bytes())
    report['config']=config
    for index in range(args.part,len(CASES),args.parts):
        case=CASES[index]
        with tempfile.TemporaryDirectory(prefix='steps-cli-contract-') as temporary:
            root=Path(temporary);write_fixture(root,case['files'])
            if args.command == 'native':
                from serverplan_steps_contract import FUNCTIONS
                from serverplan_steps_port import install
                class Violation:
                    pass
                module=types.SimpleNamespace(Violation=Violation,CAN_BATCH_FALSE_ALLOWLIST=config['canBatchAllowlist'],
                    LIFECYCLE_RUNTIME_STATE_ALLOWLIST=config['runtimeStateAllowlist'],LEGACY_SIDE_EFFECT_TAG_ALLOWLIST=config['sideEffectAllowlist'],
                    ALLOWLIST_SIGNATURE=config['signature'],POST_OP_FATE_OVERRIDES={k:types.SimpleNamespace(value=v) for k,v in config['fates'].items()})
                # Only transport constructors are passive; all semantic entries
                # are replaced, and the native run imports no source module.
                Violation.__init__=lambda *a,**k:None
                Violation.__str__=lambda *a:''
                for name in FUNCTIONS:setattr(module,name,lambda *a,**k:(_ for _ in ()).throw(AssertionError('Unported semantic entry')))
                class Backend:
                    def __init__(self):self.originals=[];self.calls=[]
                    def replace(self,owner,name,value):self.originals.append((owner,name,getattr(owner,name)));setattr(owner,name,value)
                    def native(self,check,entry,files,options=None):
                        native=execute_native(args.engine.absolute(),ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/policy_filename(check,entry),files,dict(entryPoint=entry,**(options or {})))
                        self.calls.append(dict(native,entry=entry,inputCount=len(files)));return native['diagnostics']
                backend=Backend();install(backend,module)
            dirs=[Path(p) if case.get('relative') else root/p for p in case.get('dirs',['steps'])]
            module.ROOT=root;module.STEP_DIRS=tuple(dirs);module.LEGACY_POSTOP_SCAN_DIRS=tuple(dirs)
            module.SERVERPLAN_EXTRA_SCAN_FILES=tuple(Path(p) if case.get('relative') else root/p for p in case.get('extras',[]))
            module.ALLOWLIST_SIGNATURE=case.get('signature',config['signature'])
            old=Path.cwd()
            try:
                if case.get('relative'):os.chdir(root)
                actual=invoke(module,case,root)
            finally:
                os.chdir(old)
                if args.command == 'native':
                    for owner,name,value in reversed(backend.originals):setattr(owner,name,value)
            row=dict(input=case)
            if args.command == 'source':row['expected']=actual
            else:
                row.update(expected=baseline['cases'][index]['expected'],actual=actual,nativeCalls=backend.calls)
                for call in backend.calls:
                    for item in call['programInputs']:
                        programs=report.setdefault('programs',{});programs.setdefault(item['path'],dict(sha256=item['sha256'],text=(ROOT/item['path']).read_text()))
                report['cases'].append(row);write_json(args.out,report);compare(row['expected'],row['actual']);continue
            report['cases'].append(row);write_json(args.out,report)
    report['passed']=True;write_json(args.out,report);print(dict(cases=len(report['cases']),passed=True))


if __name__ == '__main__':main()
