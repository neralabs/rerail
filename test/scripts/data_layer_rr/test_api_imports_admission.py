"""Admission must retain import/literal positives, exact types and caller scopes."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from api_imports_admission import validate
from migration import MANIFEST,read_json,require_complete


class ApiImportsAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-api-imports-complete-finalec3.json.gz','rt') as stream:cls.value=json.load(stream)

    def test_complete_contract_passes(self):
        result=validate(self.value)
        self.assertEqual(168,result['publicCasesPerAPI']);self.assertEqual(4,result['originalCasesPerAPI'])
        self.assertEqual(20,result['productionCases']);self.assertEqual(11,result['controls'])

    def test_lost_literal_positive_is_rejected(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='literal--0')
        row['actual']['value']=[]
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_incorrect_dotfile_scope_is_rejected(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='scope-.py')
        row['actual']['value']=[{}]
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_bool_line_cannot_be_replaced_with_integer(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['kind']=='from-fact' and c['input']['raw'].get('line') is True and c['expected']['value'] is not None)
        row['actual']['value']['line']=1
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_original_negative_cannot_be_omitted(self):
        value=copy.deepcopy(self.value);value['originalSuites']['snapshot']['detectorObservations'].pop()
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(value)

    def test_fixed_inventory_admits_rule(self):
        entry=next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='no_api_root_imports')
        require_complete([entry])


if __name__=='__main__':unittest.main()
