"""Supplemental reports preserve every consumed program with serializable provenance."""
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from fresh import recorded_program_inputs


class ProgramInputRecording(unittest.TestCase):
    def test_tuple_policy_keys_are_not_json_object_keys(self):
        inputs={('rule','rule.rr'):[dict(path='rule.rr',sha256='abc')]}
        actual=recorded_program_inputs(inputs)
        self.assertEqual(json.loads(json.dumps(actual)),[dict(path='rule.rr',sha256='abc')])

    def test_companions_and_shared_libraries_retain_all_inputs(self):
        a=dict(path='rule.rr',sha256='a');b=dict(path='rule-helper.rr',sha256='b');lib=dict(path='lib.rr',sha256='c')
        inputs={('rule','rule.rr'):[a,lib],('rule','rule-helper.rr'):[b,lib]}
        self.assertEqual(recorded_program_inputs(inputs),[lib,b,a])
        self.assertEqual(len(inputs[('rule','rule-helper.rr')]),2)

    def test_conflicting_revisions_cannot_be_silently_overwritten(self):
        inputs={('a','a.rr'):[dict(path='lib.rr',sha256='a')],('b','b.rr'):[dict(path='lib.rr',sha256='b')]}
        with self.assertRaisesRegex(ValueError,'Program changed'):recorded_program_inputs(inputs)

    def test_missing_optional_program_is_retained(self):
        self.assertEqual(recorded_program_inputs({('a','a.rr'):[dict(path='optional.rr',sha256=None)]}),[dict(path='optional.rr',sha256=None)])


if __name__=='__main__':unittest.main()
