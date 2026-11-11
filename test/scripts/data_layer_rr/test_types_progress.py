"""Type-safety progress must retain positives and declare every passive assertion."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from types_progress import validate


class Progress(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-type-safety-progress-finalfcce.json.gz','rt') as stream:cls.value=json.load(stream)

    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)

    def test_scoped_progress(self):
        summary=validate(self.value)
        self.assertFalse(summary['admitted'])
        self.assertEqual(summary['originalCasesPerAPI'],193)
        self.assertEqual(summary['originalPositiveObservationsPerAPI'],31)
        self.assertEqual(summary['casesPerAPI'],dict(public=99,components=95,production=15,cli=27,legacy=61))
        self.assertEqual(summary['originalConfigurationOrPassiveCasesPerAPI'],31)

    def test_omitted_positive(self):
        value=copy.deepcopy(self.value);value['originals']['snapshot'][0]['detectorObservations'].pop();self.rejected(value)

    def test_missing_original_case(self):
        value=copy.deepcopy(self.value);value['originals']['workspace'][0]['cases'].popitem();self.rejected(value)

    def test_hidden_passive_case(self):
        value=copy.deepcopy(self.value);value['originals']['workspace'][0]['casesWithoutNativeCalls']=[];self.rejected(value)

    def test_native_configuration_not_bound(self):
        value=copy.deepcopy(self.value);value['originals']['snapshot'][0]['configurationBindings']={};self.rejected(value)

    def test_missing_component(self):
        value=copy.deepcopy(self.value);value['reports']['components']['workspace'][0]['cases'].pop();self.rejected(value)

    def test_missing_native_call(self):
        value=copy.deepcopy(self.value);value['reports']['public']['snapshot'][0]['cases'][0]['native']=[];self.rejected(value)

    def test_changed_full_return(self):
        value=copy.deepcopy(self.value);value['reports']['cli']['snapshot'][0]['cases'][0]['actual']={};self.rejected(value)

    def test_different_engine(self):
        value=copy.deepcopy(self.value);value['reports']['production']['snapshot'][0]['engineSha256']='different';self.rejected(value)

    def test_different_original_policy(self):
        value=copy.deepcopy(self.value);value['originals']['snapshot'][0]['policySha256']['type_safety_patterns']='different';self.rejected(value)

    def test_source_cli_fallback(self):
        value=copy.deepcopy(self.value);value['originals']['workspace'][0]['hostProtocolCalls']=[dict(entry='cli_main')];self.rejected(value)

    def test_premature_admission(self):
        value=copy.deepcopy(self.value);value['admitted']=True;self.rejected(value)

    def test_missing_remaining_work(self):
        value=copy.deepcopy(self.value);value['remaining']=[];self.rejected(value)


if __name__=='__main__':unittest.main()
