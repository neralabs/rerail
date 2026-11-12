"""Boundary component evidence must retain complete native values and original positives."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from boundary_component_checkpoint import validate


class Components(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-boundary-components-finalfcce.json.gz','rt') as f:cls.value=json.load(f)
    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)
    def test_complete_partial_scope(self):
        s=validate(self.value);self.assertFalse(s['admitted']);self.assertEqual(s['recordedCasesPerAPI'],173);self.assertEqual(s['originalCasesPerAPI'],5);self.assertEqual(s['originalPositiveObservationsPerAPI'],3)
    def test_cannot_admit(self):
        v=copy.deepcopy(self.value);v['admitted']=True;self.rejected(v)
    def test_missing_original_positive(self):
        v=copy.deepcopy(self.value);v['originals']['workspace']['detectorObservations'].pop();self.rejected(v)
    def test_missing_component(self):
        v=copy.deepcopy(self.value);v['reports']['snapshot'][0]['cases'].pop();self.rejected(v)
    def test_changed_full_value(self):
        v=copy.deepcopy(self.value);v['reports']['workspace'][1]['cases'][0]['actual']={};self.rejected(v)
    def test_source_callback(self):
        v=copy.deepcopy(self.value);v['reports']['snapshot'][0]['semanticFallbacks']=['_violation_from_fact'];self.rejected(v)
    def test_missing_native_collection(self):
        v=copy.deepcopy(self.value)
        for report in v['reports']['workspace']:
            for row in report['cases']:
                if row['input']['id']=='extract-sync':
                    for call in row['native']:call['diagnostics']=[d for d in call['diagnostics'] if d['code']!='BS_VALUE']
        self.rejected(v)
    def test_missing_native_configuration(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot']['configurationBindings']={};self.rejected(v)
    def test_changed_program(self):
        v=copy.deepcopy(self.value);v['programs'][v['rulePath']]['text']+='\n';self.rejected(v)
    def test_another_engine(self):
        v=copy.deepcopy(self.value);v['reports']['snapshot'][0]['engineSha256']='other';self.rejected(v)


if __name__=='__main__':unittest.main()
