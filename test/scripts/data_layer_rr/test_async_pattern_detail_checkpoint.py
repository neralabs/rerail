"""Detailed async-pattern parity cannot grant admission or omit registered and raw-fact cases."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from async_pattern_detail_checkpoint import validate


class Progress(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-async-pattern-details-finalfcce.json.gz','rt') as stream:cls.value=json.load(stream)
    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)
    def test_complete_partial_scope(self):
        summary=validate(self.value);self.assertFalse(summary['admitted']);self.assertEqual(summary['recordedCasesPerAPI'],137);self.assertEqual(summary['componentCasesPerAPI'],162)
    def test_cannot_admit(self):
        v=copy.deepcopy(self.value);v['admitted']=True;self.rejected(v)
    def test_missing_raw_fact_case(self):
        v=copy.deepcopy(self.value);v['components']['reports']['workspace'][0]['cases'].pop();self.rejected(v)
    def test_changed_complete_value(self):
        v=copy.deepcopy(self.value);v['components']['reports']['snapshot'][0]['cases'][0]['actual']={};self.rejected(v)
    def test_source_fallback(self):
        v=copy.deepcopy(self.value);v['components']['reports']['workspace'][0]['semanticFallbacks']=['cli_main'];self.rejected(v)
    def test_mixed_harness(self):
        v=copy.deepcopy(self.value);v['components']['reports']['workspace'][0]['harnessFiles']['scripts/dsl/data_layer_rr/async_pattern_port.py']='different';self.rejected(v)
    def test_native_call_removed(self):
        v=copy.deepcopy(self.value);v['components']['reports']['snapshot'][0]['cases'][0]['native']=[];self.rejected(v)
    def test_missing_original_positive(self):
        v=copy.deepcopy(self.value);v['originals']['workspace']['detectorObservations'].pop();self.rejected(v)
if __name__=='__main__':unittest.main()
