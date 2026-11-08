"""An original missing-key assertion remains an exact exception observation, never an omitted case."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from fresh import invoke


class LookupExceptionTests(unittest.TestCase):
    def test_missing_key_preserves_exception_class_and_arguments(self):
        self.assertEqual(dict(error=dict(type='builtins.KeyError',args=dict(kind='tuple',items=['name']))),
            invoke(lambda:{}['name'],Path('/fixture')))

    def test_an_unexpected_harness_failure_is_not_a_source_result(self):
        def broken():
            raise RuntimeError('broken harness')
        with self.assertRaisesRegex(RuntimeError,'broken harness'):
            invoke(broken,Path('/fixture'))
