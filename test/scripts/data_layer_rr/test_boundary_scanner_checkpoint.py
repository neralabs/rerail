"""A scanner checkpoint retains all owner and file-selection witnesses without admission."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from boundary_scanner_checkpoint import validate


class Scanners(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-boundary-scanners-finalfcce.json.gz','rt') as f:cls.value=json.load(f)
    def rejected(self,v):
        with self.assertRaises(ValueError):validate(v)
    def test_complete_partial_scope(self):
        s=validate(self.value);self.assertFalse(s['admitted']);self.assertEqual(s['recordedCasesPerAPI'],173);self.assertEqual(s['scannerCasesPerAPI'],dict(public=32,production=22))
    def test_missing_owner_alias(self):
        v=copy.deepcopy(self.value)
        for r in v['scanners']['production']['reports']['workspace']:r['cases'][:]=[c for c in r['cases'] if c['input']['id']!='alias_allowed']
        self.rejected(v)
    def test_missing_public_selection(self):
        v=copy.deepcopy(self.value)
        for r in v['scanners']['public']['reports']['snapshot']:r['cases'][:]=[c for c in r['cases'] if c['input']['id']!='decode-txt']
        self.rejected(v)
    def test_changed_full_facts(self):
        v=copy.deepcopy(self.value);v['scanners']['production']['reports']['snapshot'][1]['cases'][0]['actual']={};self.rejected(v)
    def test_missing_native_work(self):
        v=copy.deepcopy(self.value);v['scanners']['public']['reports']['workspace'][0]['cases'][0]['native']=[];self.rejected(v)
    def test_source_callback(self):
        v=copy.deepcopy(self.value);v['scanners']['public']['reports']['snapshot'][0]['semanticFallbacks']=['scan_tree'];self.rejected(v)
    def test_other_rule_cohort(self):
        v=copy.deepcopy(self.value)
        for call in v['scanners']['public']['reports']['snapshot'][0]['cases'][0]['native']:
            for p in call['programInputs']:
                if p['path']==v['rulePath']:p['sha256']='different'
        self.rejected(v)
    def test_other_engine(self):
        v=copy.deepcopy(self.value);v['scanners']['production']['reports']['workspace'][0]['engineSha256']='different';self.rejected(v)


if __name__=='__main__':unittest.main()
