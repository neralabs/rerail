"""Contract errors are recorded; native assertion and runtime failures still stop acceptance."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from fresh import invoke


class FreshExceptionTests(unittest.TestCase):
    def test_value_error_preserves_type_and_arguments(self):
        def call():
            raise ValueError('AST tree is required')
        self.assertEqual({'error': {'type':'builtins.ValueError','args':{'kind':'tuple','items':['AST tree is required']}}},invoke(call,Path('/fixture')))

    def test_engine_and_assertion_failures_are_not_contract_values(self):
        for kind in (AssertionError,RuntimeError):
            def call():
                raise kind('native execution failed')
            with self.assertRaises(kind):
                invoke(call,Path('/fixture'))
