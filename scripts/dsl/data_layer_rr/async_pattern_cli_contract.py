"""Record complete async-pattern CLI streams, counters, exit values, and ratchet effects."""
import base64
import contextlib
import importlib
import io
import json
import os
from pathlib import Path
import sys
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from migration import ROOT,PIN,git
from replay import write_fixture
from async_pattern_components_contract import CHECK,BAD
from impl_direct_execution_ratchet_contract import FixedDatetime,environment

NAME='lint_async_patterns'
BASELINE='.ratchet-baselines/'+NAME+'.json'
FALLBACK='class ExampleQuery:\n    async def work(self):\n        return x.to_thread(work)\n'


def baseline(count=0,signature='UNSIGNED'):return dict(counts=dict(violations=count),_metadata=dict(signature=signature))


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    rows=[dict(id='empty',files=[]),dict(id='positive',files=[file('api/a.py')]),dict(id='summary',files=[file('api/a.py')],args=['--summary']),
        dict(id='verbose',files=[file('api/a.py')],args=['--verbose']),dict(id='actionable',files=[file('api/a.py')],args=['--actionable']),
        dict(id='fallback',files=[file('api/ExampleQuery.py',FALLBACK)]),dict(id='fallback-actionable',files=[file('api/ExampleQuery.py',FALLBACK)],args=['--actionable']),
        dict(id='mixed-summary',files=[file('api/ExampleQuery.py',FALLBACK),file('impl/sqlite/a.py')],args=['--summary']),
        dict(id='mixed-actionable-summary',files=[file('api/ExampleQuery.py',FALLBACK),file('impl/sqlite/a.py')],args=['--summary','--actionable']),
        dict(id='empty-dirs',files=[file('api/a.py')],args=['--dirs']),dict(id='duplicate-dirs',files=[file('api/a.py')],args=['--dirs','api','api']),
        dict(id='other-dir',files=[file('other/a.py')],args=['--dirs','other']),dict(id='public-token',files=[file('api/a.py'),file('impl/sqlite/a.py')],args=['--dirs','public']),
        dict(id='nested',files=[file('api/'+p+'/a.py') for p in ['normal','test','tests','tmp','venv','.venv','__pycache__','.hidden']]),
        dict(id='suffixes',files=[file('api/'+p) for p in ['a.py','a.PY','a.pyi','.py','test_a.py']]),
        dict(id='file-link',files=[file('api/a.py'),dict(path='api/link.py',symlink='a.py')]),
        dict(id='directory-link',files=[file('api/sub/a.py'),dict(path='api/link',symlink='sub')]),
        dict(id='api-interface-fails',files=[file('l2050_transaction/transaction.py','pass\n')]),
        dict(id='default-root',files=[file('api/a.py')],noRoot=True),dict(id='help',files=[],args=['--help']),dict(id='unknown-option',files=[],args=['--unknown'])]
    rows.extend(dict(id='ratchet-'+name,files=[file('api/a.py')],args=['--check',*(['--skip-signature-check'] if skip else [])],baseline=raw) for name,raw,skip in [
        ('unsigned',baseline(),False),('equal',baseline(1),True),('regression',baseline(0),True),('improvement',baseline(3),True),
        ('bad-signature',baseline(1,'bad'),True),('metadata-null',dict(counts=dict(violations=1),_metadata=None),False),('counts-array',dict(counts=[],_metadata={}),False),
        ('string-count',baseline('2'),True),('float-count',baseline(1.9),True),('boolean-count',baseline(True),True),('array-root',[],False)])
    rows += [dict(id='ratchet-missing',files=[file('api/a.py')],args=['--check']),
        dict(id='require-baseline-not-forwarded',files=[file('api/a.py')],args=['--check','--require-baseline']),
        dict(id='ratchet-malformed',files=[file('api/a.py')],args=['--check'],raw='{'),
        dict(id='ratchet-ignore-without-check',files=[file('api/a.py')],raw='{'),
        dict(id='update',files=[file('api/a.py')],args=['--update-baseline','--reason','fixture ☃']),
        dict(id='update-precedes-check',files=[file('api/a.py')],args=['--check','--update-baseline'],raw='{'),
        dict(id='sign-no-key',files=[file('api/a.py')],args=['--update-baseline','--sign']),
        dict(id='sign-public-fixture',files=[file('api/a.py')],args=['--update-baseline','--sign'],key='00'*32)]
    return [dict(kind='cli',**row) for row in rows]


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files']);discovery=importlib.import_module('scripts.lint_lib.discovery');ratchet=importlib.import_module('lint_lib.ratchet')
    script=root/'scripts/lint_lib/discovery.py';script.parent.mkdir(parents=True);script.write_text('pass\n')
    path=root/BASELINE
    if 'baseline' in case or 'raw' in case:
        path.parent.mkdir(parents=True);path.write_text(case.get('raw',json.dumps(case.get('baseline'))))
    before=base64.b64encode(path.read_bytes()).decode() if path.exists() else None
    out=io.StringIO();err=io.StringIO();argv=['lint_async_patterns.py',*([] if case.get('noRoot') else ['--root',str(root)]),*case.get('args',[])]
    with patch.dict(os.environ,environment(case),clear=True),contextlib.chdir(root),patch.object(discovery,'__file__',str(script)),patch.object(ratchet,'get_repo_root',lambda:root),patch.object(ratchet,'datetime',FixedDatetime),patch.object(sys,'argv',argv),contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
        try:value=dict(status=module.cli_main())
        except BaseException as error:value=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    after=base64.b64encode(path.read_bytes()).decode() if path.exists() else None
    return dict(value=value,stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'),baselineBefore=before,baselineAfter=after)


def install(backend,module,detectors=False):
    from async_pattern_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.async-pattern-cli/v1',cases,exercise,install,['async_pattern_cli_contract.py','async_pattern_components_contract.py','impl_direct_execution_ratchet_contract.py'],guard_modules=['lint_lib.ratchet','scripts.lint_lib.discovery'])
