"""Production policy execution must not acquire a test entryPoint option."""
import io
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
from suite import Suite


class NativeEntryTests(unittest.TestCase):
    def test_step_collectors_use_their_focused_native_policy(self):
        suite = Suite.__new__(Suite)
        suite.rule_root, suite.item = Path('/rules'), None
        suite.calls, suite.inputs = [], {}
        class Worker:
            stdin = io.StringIO()
            stdout = io.StringIO(json.dumps(dict(result=dict(programInputs=[],diagnostics=[],phases={})))+'\n')
        suite.worker = Worker()
        with patch('suite.select.select',return_value=([suite.worker.stdout],[],[])):
            suite.native('serverplan_steps','collect_current_step022_blockers',[])
        self.assertEqual('/rules/serverplan-steps-collectors.rr',json.loads(suite.worker.stdin.getvalue())['rule'])

    def test_production_and_named_entries_cross_the_same_worker_boundary(self):
        for entry in [None, 'check_file']:
            with self.subTest(entry=entry):
                suite = Suite.__new__(Suite)
                suite.rule_root = Path('/rules')
                suite.item = None
                suite.calls, suite.inputs = [], {}
                class Worker:
                    stdin = io.StringIO()
                    stdout = io.StringIO(json.dumps(dict(result=dict(programInputs=[],diagnostics=[],phases={})))+'\n')
                suite.worker = Worker()
                with patch('suite.select.select',return_value=([suite.worker.stdout],[],[])):
                    self.assertEqual([],suite.native('grammar_fragment_usage',entry,[],dict(scope='production')))
                sent = json.loads(suite.worker.stdin.getvalue())
                self.assertEqual(dict(scope='production', **({} if entry is None else dict(entryPoint=entry))),sent['options'])
