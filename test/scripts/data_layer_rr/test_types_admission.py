"""Type-safety admission requires complete originals, updates, and valid controls."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from types_admission import validate


class Admission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-type-safety-complete-finalfcce.json.gz','rt') as stream:cls.value=json.load(stream)

    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)

    def test_complete(self):
        summary=validate(self.value)
        self.assertTrue(summary['admitted'])
        self.assertEqual(summary['originalCasesPerAPI'],193)
        self.assertEqual(summary['originalPositiveObservationsPerAPI'],31)
        self.assertEqual(summary['casesPerAPI'],dict(public=99,components=95,production=15,cli=27,legacy=61))
        self.assertEqual(summary['persistentStepsPerAPI'],20)
        self.assertEqual(summary['controls'],46)

    def test_missing_positive(self):
        value=copy.deepcopy(self.value);value['originals']['workspace'][0]['detectorObservations'].pop();self.rejected(value)

    def test_hidden_passive_originals(self):
        value=copy.deepcopy(self.value);value['originals']['snapshot'][0]['casesWithoutNativeCalls']=[];self.rejected(value)

    def test_missing_legacy_case(self):
        value=copy.deepcopy(self.value);value['reports']['legacy']['snapshot'][0]['cases'].pop();self.rejected(value)

    def test_missing_alias_scope_case(self):
        value=copy.deepcopy(self.value)
        for report in value['reports']['production']['workspace']:report['cases'][:]=[r for r in report['cases'] if r['input']['id']!='alias_allowlist']
        self.rejected(value)

    def test_missing_foreign_field_case(self):
        value=copy.deepcopy(self.value)
        for report in value['reports']['components']['snapshot']:report['cases'][:]=[r for r in report['cases'] if r['input']['id']!='foreign-line-field']
        self.rejected(value)

    def test_missing_update(self):
        value=copy.deepcopy(self.value);value['updates']['snapshot'][0]['cases'].pop();self.rejected(value)

    def test_changed_noop(self):
        value=copy.deepcopy(self.value);value['updates']['workspace'][0]['cases'][2]['changedPaths']=['impl/a.py'];self.rejected(value)

    def test_weakened_update(self):
        value=copy.deepcopy(self.value);row=value['updates']['workspace'][0]['cases'][1];row['expected']=row['actual']={};self.rejected(value)

    def test_missing_control(self):
        value=copy.deepcopy(self.value);value['controls'][0]['cases'].pop();self.rejected(value)

    def test_ineffective_control(self):
        value=copy.deepcopy(self.value);row=value['controls'][0]['cases'][0];row['actual']=row['expected'];self.rejected(value)

    def test_invalid_control(self):
        value=copy.deepcopy(self.value);value['controls'][0]['cases'][0]['actual']={'error':'invalid program'};self.rejected(value)

    def test_another_engine(self):
        value=copy.deepcopy(self.value);value['updates']['snapshot'][0]['engineSha256']='other';self.rejected(value)

    def test_unrelated_mutation(self):
        value=copy.deepcopy(self.value);value['controls'][0]['cases'][0]['programCopies']={};self.rejected(value)

    def test_missing_alias_scope_control(self):
        value=copy.deepcopy(self.value)
        for report in value['controls']:report['cases'][:]=[r for r in report['cases'] if r['control']!='alias-scope']
        self.rejected(value)


if __name__=='__main__':unittest.main()
