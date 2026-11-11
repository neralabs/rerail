"""Complete ForwardRef admission must preserve native and host coverage boundaries."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from forwardref_admission import validate


class Admission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-forwardref-complete-finalfcce.json.gz','rt') as stream:cls.value=json.load(stream)

    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)

    def test_complete(self):
        summary=validate(self.value)
        self.assertTrue(summary['admitted'])
        self.assertEqual(summary['nativeLinterOriginalCasesPerAPI'],5)
        self.assertEqual(summary['hostWorkflowCasesPerAPI'],48)
        self.assertEqual(summary['originalPositiveObservationsPerAPI'],2)
        self.assertEqual(summary['casesPerAPI'],dict(public=80,components=67,production=18,cli=26,legacy=15))
        self.assertEqual(summary['persistentStepsPerAPI'],21)
        self.assertEqual(summary['controls'],24)

    def test_omitted_positive(self):
        value=copy.deepcopy(self.value);value['originals']['snapshot']['detectorObservations'].pop();self.rejected(value)

    def test_hidden_host_workflow(self):
        value=copy.deepcopy(self.value);value['originals']['workspace']['casesWithoutNativeCalls']=[];self.rejected(value)

    def test_missing_case(self):
        value=copy.deepcopy(self.value);value['reports']['components']['workspace'][0]['cases'].pop();self.rejected(value)

    def test_missing_native_call(self):
        value=copy.deepcopy(self.value);value['reports']['public']['snapshot'][0]['cases'][0]['native']=[];self.rejected(value)

    def test_changed_return(self):
        value=copy.deepcopy(self.value);value['reports']['cli']['snapshot'][0]['cases'][0]['actual']={};self.rejected(value)

    def test_mixed_engine(self):
        value=copy.deepcopy(self.value);value['reports']['legacy']['workspace'][0]['engineSha256']='other';self.rejected(value)

    def test_missing_update(self):
        value=copy.deepcopy(self.value);value['updates']['snapshot'][0]['cases'].pop();self.rejected(value)

    def test_changed_noop(self):
        value=copy.deepcopy(self.value);value['updates']['workspace'][0]['cases'][2]['changedPaths']=['impl/a.py'];self.rejected(value)

    def test_ineffective_control(self):
        value=copy.deepcopy(self.value);row=value['controls'][0]['cases'][0];row['actual']=row['expected'];self.rejected(value)

    def test_invalid_control(self):
        value=copy.deepcopy(self.value);value['controls'][0]['cases'][0]['actual']={'error':'compile failure'};self.rejected(value)

    def test_changed_program(self):
        value=copy.deepcopy(self.value);next(p for p in value['programs'].values() if p['text'])['text']+='\n';self.rejected(value)


if __name__=='__main__':unittest.main()
