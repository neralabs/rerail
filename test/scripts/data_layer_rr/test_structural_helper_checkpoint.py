"""Reject incomplete, weakened or unbound shared structural helper evidence."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from structural_helper_checkpoint import validate,POLICY


class HelperCheckpoint(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-structural-helper-complete.json.gz','rt')as stream:cls.value=json.load(stream)
    def rejected(self,v):
        with self.assertRaises(ValueError):validate(v)
    def test_complete(self):
        result=validate(self.value);self.assertEqual(result['casesPerAPI'],117);self.assertEqual(result['controls'],12)
        self.assertEqual(result['nativeExecutions'],264);self.assertFalse(result['fullRuleMigrationAcceptance'])
    def test_no_whole_rule_claim(self):
        v=copy.deepcopy(self.value);v['fullRuleMigrationAcceptance']=True;self.rejected(v)
    def test_missing_shard(self):
        v=copy.deepcopy(self.value);v['reports']['snapshot'].pop();self.rejected(v)
    def test_missing_positive(self):
        v=copy.deepcopy(self.value);v['reports']['workspace'][0]['cases'].pop();self.rejected(v)
    def test_changed_expected_and_actual(self):
        v=copy.deepcopy(self.value);r=v['reports']['snapshot'][0]['cases'][1];r['expected']=r['actual']={};self.rejected(v)
    def test_source_callback(self):
        v=copy.deepcopy(self.value);v['reports']['workspace'][0]['semanticFallbacks']=['_connection_file_violations'];self.rejected(v)
    def test_missing_native_execution(self):
        v=copy.deepcopy(self.value);v['reports']['snapshot'][0]['cases'][0]['native']=[];self.rejected(v)
    def test_wrong_program(self):
        v=copy.deepcopy(self.value);v['programs'][POLICY]['sha256']='other';self.rejected(v)
    def test_missing_control(self):
        v=copy.deepcopy(self.value);v['controls'].pop();self.rejected(v)
    def test_ineffective_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['actual']=v['controls'][0]['expected'];self.rejected(v)
    def test_invalid_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['actual']=dict(error='compile failure');self.rejected(v)
    def test_unrelated_mutation(self):
        v=copy.deepcopy(self.value);v['controls'][0]['programCopies']={};self.rejected(v)
    def test_changed_source_recording(self):
        v=copy.deepcopy(self.value);v['source']['text']='{}';self.rejected(v)
    def test_changed_absent_dependency(self):
        v=copy.deepcopy(self.value);absent=next(r for r in v['programs'].values()if r['sha256']is None);absent['text']='fake';self.rejected(v)


if __name__=='__main__':unittest.main()
