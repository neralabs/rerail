"""Acceptance modes must execute their declared engine API and reject mismatched worker provenance."""
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import replay
from suite import Suite


class ExecutionModeTests(unittest.TestCase):
    def request(self):
        return dict(rule='/rules/a.rr',files=[dict(path='input.py',source='pass\n')],options=dict(entryPoint='scan_source'))

    def test_each_mode_calls_its_real_api_and_records_the_mode(self):
        for mode in ['snapshot','workspace']:
            with self.subTest(mode=mode):
                out=io.StringIO();request=self.request();result=dict(diagnostics=[],programInputs=[],phases=[])
                with patch.object(replay,'execute_native',return_value=result) as snapshot,patch.object(replay,'execute_workspace',return_value=result) as workspace,patch.object(sys,'stdin',io.StringIO(json.dumps(request)+'\n')),patch.object(sys,'stdout',out):
                    replay.worker(Path('/engine'),mode)
                chosen,unused=(snapshot,workspace) if mode=='snapshot' else (workspace,snapshot)
                chosen.assert_called_once_with(Path('/engine'),Path(request['rule']),request['files'],request['options'])
                unused.assert_not_called()
                self.assertEqual(json.loads(out.getvalue()),dict(result=result,executionMode=mode))

    def test_unknown_mode_never_reaches_an_engine(self):
        with patch.object(replay,'execute_native') as snapshot,patch.object(replay,'execute_workspace') as workspace,self.assertRaises(ValueError):
            replay.worker(Path('/engine'),'pretend')
        snapshot.assert_not_called();workspace.assert_not_called()

    def test_a_snapshot_response_cannot_certify_workspace_execution(self):
        suite=Suite.__new__(Suite);suite.rule_root=Path('/rules');suite.item=None;suite.calls=[];suite.inputs={};suite.execution_mode='workspace'
        suite.worker=type('Worker',(),dict(stdin=io.StringIO(),stdout=io.StringIO(json.dumps(dict(executionMode='snapshot',result=dict(programInputs=[],diagnostics=[],phases=[])))+'\n')))()
        with patch('suite.select.select',return_value=([suite.worker.stdout],[],[])),self.assertRaisesRegex(ValueError,'execution mode'):
            suite.native('execution_authority_bypass','scan_source',[])
