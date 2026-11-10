"""Admission must retain name-family positives, iterable semantics and alias findings."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from name_inference_admission import validate
from migration import MANIFEST,read_json,require_complete


class NameInferenceAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-name-inference-complete-finalec3.json.gz','rt') as stream:cls.value=json.load(stream)

    def test_complete_contract_passes(self):
        result=validate(self.value)
        self.assertEqual(193,result['publicCasesPerAPI']);self.assertEqual(8,result['originalCasesPerAPI'])
        self.assertEqual(7,result['originalFindingPositiveCases']);self.assertEqual(29,result['productionCases'])
        self.assertEqual(541,result['nativeExecutions'])

    def test_reference_positive_cannot_be_dropped(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='parse-.raw-endswith')
        row['actual']['value']=[]
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_empty_generator_does_not_request_defaults(self):
        value=copy.deepcopy(self.value)
        rows={c['input']['id']:c for r in value['publicReports'] if r['executionMode']=='snapshot' for c in r['cases']}
        self.assertTrue(rows['violations-empty-list']['actual']['value'])
        self.assertEqual([],rows['violations-empty-generator']['actual']['value'])
        rows['violations-empty-generator']['actual']['value']=rows['violations-empty-list']['actual']['value']
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_duplicate_basenames_are_not_a_set(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='violations-basename-collision')
        self.assertEqual(2,len(row['actual']['value']));row['actual']['value'].pop()
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_filename_alias_positive_cannot_be_dropped(self):
        value=copy.deepcopy(self.value)
        row=next(c for c in value['productionSupplement']['report']['cases'] if c['input']['id'].endswith(':type-alias'))
        self.assertEqual(1,len(row['actual']['findings']));row['actual']['findings']=[]
        with self.assertRaisesRegex(ValueError,'Production trigger result'):validate(value)

    def test_fixed_inventory_admits_rule(self):
        entry=next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='serverplan_name_inference')
        require_complete([entry])


if __name__=='__main__':unittest.main()
