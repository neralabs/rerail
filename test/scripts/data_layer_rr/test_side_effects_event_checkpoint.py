"""Admission evidence cannot omit callback positives or split a declared batch."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from side_effects_event_checkpoint import validate


class SideEffectsEventCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-side-effects-events-current.json.gz','rt')as stream:cls.receipt=json.load(stream)

    def test_complete_unchanged_and_supplemental_contracts_pass_both_apis(self):
        self.assertEqual(validate(self.receipt),dict(originalAssertionsPerAPI=7,comparisonsPerAPI=549,callbackBatch=64,fullMigrationAcceptance=False))

    def test_omitting_a_callback_case_is_rejected(self):
        value=copy.deepcopy(self.receipt);value['contracts']['incremental']['workspace'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'Native case omitted'):validate(value)

    def test_a_changed_finding_is_rejected(self):
        value=copy.deepcopy(self.receipt);report=value['contracts']['incremental']['snapshot'][0]
        row=next(r for r in report['cases']if r['input']['id']=='batched-scope');row['actual']['facts'].pop()
        with self.assertRaisesRegex(ValueError,'Complete native result differs'):validate(value)

    def test_per_callback_executions_are_rejected(self):
        value=copy.deepcopy(self.receipt);report=value['contracts']['incremental']['snapshot'][0]
        row=next(r for r in report['cases']if r['input']['id']=='batched-scope');row['native']*=2
        with self.assertRaisesRegex(ValueError,'batch split'):validate(value)

    def test_an_omitted_original_assertion_is_rejected(self):
        value=copy.deepcopy(self.receipt);value['originals']['workspace']['cases'].pop(next(iter(value['originals']['workspace']['cases'])))
        with self.assertRaisesRegex(ValueError,'Original assertion or observation omitted'):validate(value)

    def test_a_subset_cannot_be_claimed_as_full_rule_admission(self):
        value=copy.deepcopy(self.receipt);value['fullMigrationAcceptance']=True
        with self.assertRaisesRegex(ValueError,'not full rule admission'):validate(value)

    def test_consumed_harness_bytes_cannot_be_omitted(self):
        value=copy.deepcopy(self.receipt)
        value['snapshots']=[r for r in value['snapshots']if r['path']!='scripts/dsl/data_layer_rr/ast_transport.py']
        with self.assertRaisesRegex(ValueError,'Consumed bytes omitted'):validate(value)

    def test_native_positive_witness_cannot_be_omitted(self):
        value=copy.deepcopy(self.receipt);report=value['contracts']['incremental']['workspace'][0]
        row=next(r for r in report['cases']if r['input']['id']=='batched-scope')
        findings=row['native'][0]['diagnostics'];findings.pop(next(i for i,r in enumerate(findings)if r['code']=='PSE_EVENT_FACT'))
        with self.assertRaisesRegex(ValueError,'Native batch witness missing'):validate(value)


if __name__=='__main__':unittest.main()
