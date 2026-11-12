"""Async-pattern progress retains exact production and supplemental parity without admission."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from async_pattern_checkpoint import validate


class Progress(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-async-pattern-owner-progress-finalfcce.json.gz','rt') as stream:cls.value=json.load(stream)

    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)

    def test_exact_partial_scope(self):
        summary=validate(self.value)
        self.assertFalse(summary['admitted'])
        self.assertEqual(summary['originalCasesPerAPI'],11)
        self.assertEqual(summary['originalPositiveObservationsPerAPI'],8)
        self.assertEqual(summary['recordedCasesPerAPI'],19)
        self.assertEqual(summary['supplementalCasesSnapshot'],dict(api=158,report=136))

    def test_cannot_claim_admission(self):
        value=copy.deepcopy(self.value);value['admitted']=True;self.rejected(value)

    def test_missing_alias_case(self):
        value=copy.deepcopy(self.value)
        for report in value['reports']['workspace']:report['cases'][:]=[r for r in report['cases'] if r['input']['id']!='alias_scope']
        self.rejected(value)

    def test_missing_supplemental_case(self):
        value=copy.deepcopy(self.value);value['supplemental']['api']['reports'][0]['results'].pop();self.rejected(value)

    def test_mixed_native_program(self):
        value=copy.deepcopy(self.value);value['supplemental']['api']['reports'][0]['programInputs'][0]['sha256']='other';self.rejected(value)

    def test_false_clean_result(self):
        value=copy.deepcopy(self.value)
        item=value['supplemental']['api']['reports'][0]['results'][0];item['value']={'result':[]};self.rejected(value)

    def test_shared_reporter_cache(self):
        value=copy.deepcopy(self.value);value['supplemental']['report']['reports'][0].pop('executionIsolation');self.rejected(value)

    def test_unfinished_supplemental_run(self):
        value=copy.deepcopy(self.value);value['supplemental']['api']['reports'][0]['status']='started';self.rejected(value)


if __name__=='__main__':unittest.main()
