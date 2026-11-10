"""Infrastructure messages are complete public return values, not invented findings."""
from collections import Counter
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch
ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from suite import Suite
import data_layer_record


class BlanketObservationTests(unittest.TestCase):
    def test_missing_file_compares_the_complete_recorded_string_result(self):
        suite=Suite.__new__(Suite);identity='missing';result=['  missing.py: file not found'];scope=dict(include=['missing.py'],exclude=[])
        suite.item=SimpleNamespace(nodeid=identity,funcargs={});suite.observed=Counter();suite.observations=[]
        suite.baseline=dict(events=[dict(test=identity,check='no_blanket_grammar_bypass',entry='check_no_public_grammar_methods',
            files=[],scope=scope,sourceResult=result,retained_reason='Output adapter: CLI infrastructure diagnostic')])
        with patch.object(data_layer_record,'snapshot',return_value=([],scope,None)):
            suite.compare_original('no_blanket_grammar_bypass','check_no_public_grammar_methods',SimpleNamespace(),(Path('/project'),),{},result)
        row=suite.observations[0]
        self.assertEqual('passed',row['status']);self.assertEqual('source-value',row['comparisonKind'])
        self.assertEqual(result,row['expected']);self.assertEqual(result,row['actual'])

    def test_empty_tree_scan_keeps_a_complete_observation(self):
        suite=Suite.__new__(Suite);identity='empty';scope=dict(include=['**/*.py'],exclude=[])
        suite.item=SimpleNamespace(nodeid=identity,funcargs={});suite.observed=Counter();suite.observations=[]
        suite.baseline=dict(events=[dict(test=identity,check='no_blanket_grammar_bypass',entry='check_no_bypass_references',
            files=[],scope=scope,sourceResult=[],findings=[],retained_reason='No readable source files')])
        with patch.object(data_layer_record,'snapshot',return_value=([],scope,None)):
            suite.compare_original('no_blanket_grammar_bypass','check_no_bypass_references',SimpleNamespace(),(Path('/project'),),{},[])
        self.assertEqual('passed',suite.observations[0]['status'])
