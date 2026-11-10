"""Admission must reject omitted assertions, mixed cohorts and ineffective controls."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).absolute().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from permission_admission import validate
from migration import read_json, MANIFEST, require_complete


class PermissionAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-permission-complete-finalec3.json.gz','rt') as stream:
            cls.value = json.load(stream)

    def test_all_recorded_families_pass(self):
        result = validate(self.value)
        self.assertEqual(116,result['publicCasesPerAPI'])
        self.assertEqual(5,result['originalCasesPerAPI'])
        self.assertEqual(6,result['controls'])
        self.assertGreater(result['nativeExecutions'],300)

    def test_missing_public_case_is_rejected(self):
        value = copy.deepcopy(self.value); value['publicReports'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'omitted or repeated'): validate(value)

    def test_mixed_engine_is_rejected(self):
        value = copy.deepcopy(self.value); value['publicReports'][0]['engineSha256'] = '0'*64
        with self.assertRaisesRegex(ValueError,'Mixed engine'): validate(value)

    def test_missing_positive_is_rejected(self):
        value = copy.deepcopy(self.value)
        row = next(r for r in value['publicReports'][0]['cases'] if r['input']['kind']=='from-fact' and r['expected']['value'])
        row['actual']['value'] = None
        with self.assertRaisesRegex(ValueError,'Public result'): validate(value)

    def test_original_skip_is_rejected(self):
        value = copy.deepcopy(self.value)
        phases = next(iter(value['originalSuites']['snapshot']['cases'].values()))
        next(p for p in phases if p['phase']=='call')['outcome'] = 'skipped'
        with self.assertRaisesRegex(ValueError,'failed or skipped'): validate(value)

    def test_control_must_fail_for_the_intended_reason(self):
        value = copy.deepcopy(self.value)
        value['reporterControl']['cases'][0]['actual'] = dict(error='unrelated error')
        with self.assertRaisesRegex(ValueError,'unrelated reason'): validate(value)

    def test_consumed_program_must_match(self):
        value = copy.deepcopy(self.value)
        value['publicReports'][0]['cases'][0]['native'][0]['programInputs'][0]['sha256'] = '0'*64
        with self.assertRaisesRegex(ValueError,'Consumed program'): validate(value)

    def test_fixed_original_inventory_admits_permission(self):
        entry = next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='permission_error')
        require_complete([entry])


if __name__=='__main__': unittest.main()
