"""Dispatch progress cannot hide positives, partial runs, or altered oracle values."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from dispatch_checkpoint import validate


class DispatchCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-dispatch-cli-finald03-progress.json.gz','rt') as stream:cls.value=json.load(stream)

    def report(self,value,kind='cli',mode='snapshot'):
        return next(value['reports'][name] for name,pair in value['reportKinds'].items() if pair==[kind,mode])

    def test_complete_recorded_progress_retains_its_admission_gaps(self):
        result=validate(self.value)
        self.assertFalse(result['admitted'])
        self.assertEqual(result['casesPerAPI'],dict(original=49,scanner=298,finding=47,supplied=222,repository=43,updates=17,cli=48,signing=31))
        self.assertTrue(self.value['pending'])

    def test_missing_positive_cli_case_is_rejected(self):
        value=copy.deepcopy(self.value)
        report=next(r for name,r in value['reports'].items() if value['reportKinds'][name]==['cli','snapshot'] and any(c['input']['id']=='real-positive' for c in r['cases']))
        report['cases']=[c for c in report['cases'] if c['input']['id']!='real-positive']
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(value)

    def test_partial_report_cannot_claim_success(self):
        value=copy.deepcopy(self.value);self.report(value)['complete']=False
        with self.assertRaisesRegex(ValueError,'Interrupted'):validate(value)

    def test_boolean_exit_status_is_not_an_integer_exit_status(self):
        value=copy.deepcopy(self.value)
        row=next(c for name,r in value['reports'].items() if value['reportKinds'][name]==['cli','snapshot'] for c in r['cases'] if c['actual'].get('status')==0)
        row['actual']['status']=False
        with self.assertRaisesRegex(ValueError,'typed result'):validate(value)

    def test_expected_and_actual_cannot_be_changed_together(self):
        value=copy.deepcopy(self.value);row=self.report(value)['cases'][0]
        row['actual']['stdout']='invented';row['expected']['stdout']='invented'
        with self.assertRaisesRegex(ValueError,'oracle'):validate(value)

    def test_oracle_contents_must_match_the_recorded_bytes(self):
        value=copy.deepcopy(self.value)
        value['sourceRecordings']['cli']['cases'][0]['expected']['stdout']='invented'
        with self.assertRaisesRegex(ValueError,'Source recording contents'):validate(value)

    def test_missing_original_assertion_is_rejected(self):
        value=copy.deepcopy(self.value);self.report(value,'original')['cases'].popitem()
        with self.assertRaisesRegex(ValueError,'Original test coverage'):validate(value)

    def test_wrong_consumed_program_hash_is_rejected(self):
        value=copy.deepcopy(self.value)
        native=next(n for c in self.report(value)['cases'] for n in c['native'])
        native['programInputs'][0]['sha256']='0'*64
        with self.assertRaisesRegex(ValueError,'Consumed program'):validate(value)


if __name__=='__main__':unittest.main()
