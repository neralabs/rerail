"""Acceptance evidence must reject omissions, type coercion, and changed programs."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from validate_supplied_facts_checkpoint import validate


class SuppliedFactsCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-nine-supplied-fact-reporters-finald024.json.gz','rt') as stream:
            cls.receipt=json.load(stream)

    def setUp(self): self.value=copy.deepcopy(self.receipt)

    def test_complete_receipt(self):
        self.assertEqual(153,validate(self.value)['nativeExecutions'])

    def test_missing_or_duplicate_case(self):
        for duplicate in [False,True]:
            value=copy.deepcopy(self.value); rows=value['reports'][0]['cases']; lost=rows.pop()
            if duplicate: rows.append(copy.deepcopy(rows[0]))
            with self.subTest(duplicate=duplicate),self.assertRaisesRegex(ValueError,'omitted or repeated'): validate(value)

    def test_boolean_line_cannot_be_replaced_with_integer(self):
        row=next(r for report in self.value['reports'] for r in report['cases'] if r['id']=='backend_type_strings:9')
        finding=next(f for f in row['actual']['findings'] if f['line'] is True); finding['line']=1
        with self.assertRaisesRegex(ValueError,'Complete reporter result differs'): validate(self.value)

    def test_controls_must_fail_for_the_disabled_reporter(self):
        self.value['controls']['cases'][0]['actual']={'error':'unrelated exception'}
        with self.assertRaisesRegex(ValueError,'unrelated reason'): validate(self.value)

    def test_missing_positive_control(self):
        self.value['controls']['cases'].pop()
        with self.assertRaisesRegex(ValueError,'controls omitted or repeated'): validate(self.value)

    def test_engine_and_program_hashes_are_bound(self):
        self.value['reports'][0]['engineSha256']='wrong'
        with self.assertRaisesRegex(ValueError,'Native engine differs'): validate(self.value)
        self.value=copy.deepcopy(self.receipt)
        next(iter(self.value['programs'].values()))['text']+='\n// changed\n'
        with self.assertRaisesRegex(ValueError,'snapshot differs'): validate(self.value)


if __name__=='__main__': unittest.main()
