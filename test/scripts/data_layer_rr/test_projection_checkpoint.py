"""Projection evidence must retain coercions, errors, multiplicity and native calls."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from validate_projection_checkpoint import validate


class ProjectionCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-model-projection-final041.json.gz','rt') as stream:cls.receipt=json.load(stream)

    def setUp(self):self.value=copy.deepcopy(self.receipt)

    def test_complete_receipt(self):self.assertEqual(206,validate(self.value)['projectionCases'])

    def test_missing_or_repeated_case_is_rejected(self):
        self.value['reports'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(self.value)

    def test_fields_present_in_the_input_are_part_of_the_contract(self):
        row=next(r for report in self.value['reports'] for r in report['cases'] if r['input']['id']=='projection-models:7.0')
        row['actual']['model']['values']['files']['tuple'][0]['fieldsSet'].append('syntax_error')
        with self.assertRaisesRegex(ValueError,'Complete model or message result differs'):validate(self.value)

    def test_coercion_control_requires_the_expected_protocol_failure(self):
        self.value['controls']['cases'][0]['actual']={'unrelated':'exception'}
        with self.assertRaisesRegex(ValueError,'unrelated reason'):validate(self.value)

    def test_original_cli_must_call_native_projection(self):
        r=self.value['originalSuites']['workspace'];r['calls']=[c for c in r['calls'] if c['entry']!='_project_facts']
        with self.assertRaisesRegex(ValueError,'native projection'):validate(self.value)

    def test_program_and_engine_identity_are_bound(self):
        self.value['reports'][0]['engineSha256']='wrong'
        with self.assertRaisesRegex(ValueError,'Invalid native cohort'):validate(self.value)
        self.value=copy.deepcopy(self.receipt);next(iter(self.value['programs'].values()))['text']+='\n// changed\n'
        with self.assertRaisesRegex(ValueError,'snapshot differs'):validate(self.value)


if __name__=='__main__':unittest.main()
