"""Frontend parity retains complete outputs, errors and baseline side effects without admission."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from async_pattern_frontend_checkpoint import validate


class Progress(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-async-pattern-frontends-finalfcce.json.gz','rt') as f:cls.value=json.load(f)
    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)
    def test_complete_partial_scope(self):
        s=validate(self.value);self.assertFalse(s['admitted']);self.assertEqual(s['casesPerAPI'],dict(files=28,cli=40))
    def test_cannot_admit(self):
        v=copy.deepcopy(self.value);v['admitted']=True;self.rejected(v)
    def test_missing_cli_case(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['snapshot'][0]['cases']=[];self.rejected(v)
    def test_source_cli_fallback(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['workspace'][0]['semanticFallbacks']=['cli_main'];self.rejected(v)
    def test_weakened_stdout(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['snapshot'][1]['cases'][0]['actual']['stdout']='';self.rejected(v)
    def test_missing_native_counter(self):
        v=copy.deepcopy(self.value);v['groups']['files']['reports']['snapshot'][0]['cases'][0]['native']=[];self.rejected(v)
    def test_changed_baseline_write(self):
        v=copy.deepcopy(self.value)
        for report in v['groups']['cli']['reports']['workspace']:
            for row in report['cases']:
                if row['input']['id']=='update':row['actual']['baselineAfter']=None
        self.rejected(v)
    def test_changed_program(self):
        v=copy.deepcopy(self.value);v['programs'][v['rulePath']]['sha256']='different';self.rejected(v)
if __name__=='__main__':unittest.main()
