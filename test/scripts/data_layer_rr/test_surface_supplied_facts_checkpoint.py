"""The Surface receipt must bind malformed inputs, positives and original assertions."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from validate_surface_supplied_facts_checkpoint import validate


class SurfaceSuppliedFactsCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-surface-supplied-facts-final957.json.gz','rt') as stream:
            cls.receipt=json.load(stream)

    def setUp(self):self.value=copy.deepcopy(self.receipt)

    def test_complete_receipt(self):
        result=validate(self.value)
        self.assertEqual((197,124,18),(result['suppliedCases'],result['expectedValidationErrors'],result['originalAssertions']))

    def test_missing_or_duplicate_source_case(self):
        self.value['reports'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(self.value)

    def test_validation_error_order_is_part_of_parity(self):
        row=next(r for report in self.value['reports'] for r in report['cases']
                 if len(r['actual'].get('error',{}).get('details',[]))>1)
        row['actual']['error']['details'].reverse()
        with self.assertRaisesRegex(ValueError,'Complete native result differs'):validate(self.value)

    def test_unrelated_control_exception_is_not_a_pass(self):
        self.value['controls']['cases'][0]['actual']={'error':'unrelated exception'}
        with self.assertRaisesRegex(ValueError,'unrelated reason'):validate(self.value)

    def test_original_plugin_positive_must_run_natively(self):
        report=self.value['originalSuites']['workspace']
        report['calls']=[c for c in report['calls'] if c['entry']!='_report_facts']
        with self.assertRaisesRegex(ValueError,'positive did not run natively'):validate(self.value)

    def test_changed_program_or_engine_is_rejected(self):
        next(iter(self.value['programs'].values()))['text']+='\n// changed\n'
        with self.assertRaisesRegex(ValueError,'snapshot differs'):validate(self.value)
        self.value=copy.deepcopy(self.receipt);self.value['reports'][0]['engineSha256']='wrong'
        with self.assertRaisesRegex(ValueError,'Native cohort differs'):validate(self.value)


if __name__=='__main__':unittest.main()
