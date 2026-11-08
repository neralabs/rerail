"""A native uncaught-config result must preserve the source reporter exception contract."""
import json
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import reporting
from migration import PIN,sha


class ReporterExceptionTests(unittest.TestCase):
    def test_native_missing_key_is_an_exception_observation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            cases=root/'cases.json';cases.write_text(json.dumps(dict(cases=[dict(id='missing-name',check='module_level_caches',files=[])])))
            expected=dict(error=dict(type='builtins.KeyError',args=dict(kind='tuple',items=['name'])))
            oracle=root/'oracle.json';oracle.write_text(json.dumps(dict(sourcePin=PIN,execution='offline-source',
                casesSha256=sha(cases.read_bytes()),results=[dict(id='missing-name',value=expected,status='passed')])) )
            engine=root/'engine';engine.write_text('test executable')
            native=dict(metadata=dict(policies=[dict(id='data-layer-module-level-caches')],settings=[['mode','block']]),
                diagnostics=[dict(clause='MCUncaughtAllowlist',code='MC_UNCAUGHT_CONFIG',path='allowlist.json',line=1,
                    message='name',parameters=dict(kind='KeyError',value='name'))],programInputs=[],phases=[])
            args=SimpleNamespace(command='native',cases=cases,oracle=oracle,engine=engine,rule_root=root,part=0,parts=1,out=root/'out.json')
            with patch.object(reporting,'execute_native',return_value=native):
                result=reporting.run(args)
            self.assertEqual(expected,result['results'][0]['value'])
