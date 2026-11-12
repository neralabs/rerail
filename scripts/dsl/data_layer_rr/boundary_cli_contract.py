"""Record boundary CLI output, exit values, discovery, context and baseline byte changes."""
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
from replay import write_fixture
from boundary_production_contract import CHECK,BAD
from impl_direct_execution_ratchet_contract import FixedDatetime,environment

NAME='lint_async_sync_boundary'
BASELINE='.ratchet-baselines/'+NAME+'.json'


def baseline(count=0,signature='UNSIGNED'):return dict(counts=dict(actionable=count),_metadata=dict(signature=signature))


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    rows=[dict(id='empty',files=[]),dict(id='positive',files=[file('impl/a.py')]),dict(id='context',files=[file('impl/a.py')],args=['--context']),
        dict(id='allowed',files=[file('impl/sqlite/native_async_wrapper.py')]),dict(id='show-allowed',files=[file('impl/sqlite/native_async_wrapper.py')],args=['--show-allowlisted']),
        dict(id='informational',files=[file('impl/tests/a.py')]),dict(id='all-categories',files=[file('impl/a.py',BAD+'async def awork(loop):\n    await to_thread(work)\n    await loop.run_in_executor(None,work)\n')]),
        dict(id='mixed',files=[file('impl/a.py'),file('impl/tests/a.py'),file('impl/sqlite/native_async_wrapper.py')],args=['--show-allowlisted']),
        dict(id='default-root',files=[file('impl/a.py')],noRoot=True),dict(id='empty-dirs',files=[file('impl/a.py')],args=['--dirs']),
        dict(id='duplicate-dirs',files=[file('impl/a.py')],args=['--dirs','impl','impl']),dict(id='custom-dir',files=[file('other/a.py')],args=['--dirs','other']),
        dict(id='deprecated-path',files=[file('other/a.py')],args=['--path','other']),
        dict(id='path-overrides-dirs',files=[file('other/a.py')],args=['--path','other','--dirs']),
        dict(id='suffixes',files=[file('impl/'+p) for p in ['.py','a.py','a.PY','a.pyi','test_a.py']]),
        dict(id='nested',files=[file('impl/'+p+'/a.py') for p in ['normal','test','tests','tmp','venv','.venv','__pycache__','.hidden']]),
        dict(id='help',files=[],args=['--help']),dict(id='unknown-option',files=[],args=['--unknown'])]
    rows.extend(dict(id='ratchet-'+name,files=[file('impl/a.py')],args=['--check',*(['--skip-signature-check'] if skip else [])],baseline=raw) for name,raw,skip in [
        ('unsigned',baseline(),False),('equal',baseline(1),True),('regression',baseline(0),True),('improvement',baseline(3),True),
        ('bad-signature',baseline(1,'bad'),True),('metadata-null',dict(counts=dict(actionable=1),_metadata=None),False),('counts-array',dict(counts=[],_metadata={}),False),
        ('string-count',baseline('2'),True),('float-count',baseline(1.9),True),('boolean-count',baseline(True),True),('array-root',[],False)])
    rows += [dict(id='ratchet-missing',files=[file('impl/a.py')],args=['--check']),dict(id='ratchet-malformed',files=[file('impl/a.py')],args=['--check'],raw='{'),
        dict(id='require-baseline-not-forwarded',files=[file('impl/a.py')],args=['--check','--require-baseline']),
        dict(id='clean-check',files=[],args=['--check']),dict(id='update',files=[file('impl/a.py')],args=['--update-baseline','--reason','fixture ☃']),
        dict(id='update-precedes-check',files=[file('impl/a.py')],args=['--check','--update-baseline'],raw='{'),dict(id='sign-no-key',files=[file('impl/a.py')],args=['--update-baseline','--sign']),
        dict(id='sign-public-fixture',files=[file('impl/a.py')],args=['--update-baseline','--sign'],key='00'*32)]
    return [dict(kind='cli',**r) for r in rows]


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    discovery=importlib.import_module('scripts.lint_lib.discovery');ratchet=importlib.import_module('lint_lib.ratchet')
    script=root/'scripts/lint_lib/discovery.py';script.parent.mkdir(parents=True);script.write_text('pass\n')
    path=root/BASELINE
    if 'baseline' in case or 'raw' in case:path.parent.mkdir(parents=True);path.write_text(case.get('raw',json.dumps(case.get('baseline'))))
    before=base64.b64encode(path.read_bytes()).decode() if path.exists() else None
    out=io.StringIO();err=io.StringIO();argv=['lint_async_sync_boundary.py',*([] if case.get('noRoot') else ['--root',str(root)]),*case.get('args',[])]
    with patch.dict(os.environ,environment(case),clear=True),contextlib.chdir(root),patch.object(module,'__file__',str(root/'scripts/lint_async_sync_boundary.py')),patch.object(discovery,'__file__',str(script)),patch.object(ratchet,'get_repo_root',lambda:root),patch.object(ratchet,'datetime',FixedDatetime),patch.object(sys,'argv',argv),contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
        try:value=dict(status=module.cli_main())
        except BaseException as error:value=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    after=base64.b64encode(path.read_bytes()).decode() if path.exists() else None
    return dict(value=value,stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'),baselineBefore=before,baselineAfter=after)


def install(backend,module,detectors=False):
    from boundary_components_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.boundary-cli/v1',cases,exercise,install,['boundary_cli_contract.py','boundary_production_contract.py','impl_direct_execution_ratchet_contract.py'],guard_modules=['lint_lib.ratchet','scripts.lint_lib.discovery'])
