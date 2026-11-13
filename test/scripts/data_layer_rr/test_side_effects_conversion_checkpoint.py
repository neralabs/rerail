"""Conversion evidence must retain values, exceptions, and every source case."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from side_effects_conversion_checkpoint import validate


class SideEffectsConversionCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-side-effects-conversion-current.json.gz','rt')as stream:
            cls.receipt=json.load(stream)

    def case(self,value,name,mode='snapshot'):
        return next(row for report in value['native'][mode]for row in report['cases']if row['input']['id']==name)

    def test_all_conversion_observations_pass_both_apis(self):
        self.assertEqual(validate(self.receipt),dict(conversionCasesPerAPI=105,captureDemandCasesPerAPI=3,fullMigrationAcceptance=False))

    def test_unread_payload_cannot_remove_a_valid_fact(self):
        value=copy.deepcopy(self.receipt);value['captureDemand']['native']['workspace']['cases'][0]['actual']={'value':None}
        with self.assertRaisesRegex(ValueError,'Complete capture-demand observation differs'):validate(value)

    def test_omitted_case_is_rejected(self):
        value=copy.deepcopy(self.receipt);value['native']['workspace'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'Conversion case omitted'):validate(value)

    def test_valid_cast_cannot_disappear(self):
        value=copy.deepcopy(self.receipt);self.case(value,'fact-line-5')['actual']={'value':None}
        with self.assertRaisesRegex(ValueError,'Complete conversion observation differs'):validate(value)

    def test_boolean_line_cannot_be_replaced_by_integer(self):
        value=copy.deepcopy(self.receipt);self.case(value,'structured-line-0')['actual']['value']['fields']['line_number']=1
        with self.assertRaisesRegex(ValueError,'Complete conversion observation differs'):validate(value)

    def test_infinity_error_cannot_be_swallowed(self):
        value=copy.deepcopy(self.receipt);self.case(value,'fact-inf')['actual']={'value':None}
        with self.assertRaisesRegex(ValueError,'Complete conversion observation differs'):validate(value)

    def test_final_apis_cannot_mix_program_revisions(self):
        value=copy.deepcopy(self.receipt)
        row=self.case(value,'fact-line-0','workspace')
        item=next(item for call in row['native']for item in call['programInputs']if item['path']=='docs/rule-syntax/lib/data-layer/side-effects-from-fact.rr')
        item['sha256']='f'*64
        with self.assertRaisesRegex(ValueError,'Final program revisions differ'):validate(value)

    def test_consumed_integer_policy_bytes_cannot_be_omitted(self):
        value=copy.deepcopy(self.receipt)
        value['snapshots']=[row for row in value['snapshots']if row['path']!='docs/rule-syntax/lib/data-layer/side-effects-from-fact.rr']
        with self.assertRaisesRegex(ValueError,'Consumed bytes omitted'):validate(value)

    def test_native_cast_capability_failure_must_be_retained(self):
        value=copy.deepcopy(self.receipt);value['tdd']['missingCast']['cases'][0]['actual']={'value':None}
        with self.assertRaisesRegex(ValueError,'Missing execution-capability failure'):validate(value)

    def test_progress_cannot_claim_full_admission(self):
        value=copy.deepcopy(self.receipt);value['fullMigrationAcceptance']=True
        with self.assertRaisesRegex(ValueError,'not full rule admission'):validate(value)


if __name__=='__main__':unittest.main()
