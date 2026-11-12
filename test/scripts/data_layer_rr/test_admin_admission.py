"""Require exact whole-contract admin evidence before admission."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from admin_admission import validate


class Admission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-admin-complete-finalfcce.json.gz','rt') as stream:cls.value=json.load(stream)
    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)
    def test_complete(self):
        s=validate(self.value);self.assertTrue(s['admitted']);self.assertTrue(s['fullRuleMigrationAcceptance']);self.assertFalse(s['fullMigrationAcceptance'])
        self.assertEqual(s['originalCasesPerAPI'],125);self.assertEqual(s['originalObservationsPerAPI'],51);self.assertEqual(s['originalPositiveObservationsPerAPI'],26)
        self.assertEqual(s['casesPerAPI'],dict(components=128,public=38,cli=31,production=22,legacy=59));self.assertEqual(s['persistentStepsPerAPI'],21);self.assertEqual(s['controls'],36)
    def test_missing_original_case(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot'][0]['cases'].popitem();self.rejected(v)
    def test_missing_original_positive(self):
        v=copy.deepcopy(self.value)
        for r in v['originals']['workspace']:
            if r['detectorObservations']:r['detectorObservations'].pop();break
        self.rejected(v)
    def test_changed_original_dto(self):
        v=copy.deepcopy(self.value)
        for r in v['originals']['snapshot']:
            if r['detectorObservations']:r['detectorObservations'][0]['actualSourceResult']=None;break
        self.rejected(v)
    def test_unbound_constants(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot'][0]['configurationBindings']['admin_async_purity']['names'].remove('FORBIDDEN_PATTERNS');self.rejected(v)
    def test_host_cli(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot'][0]['hostProtocolCalls']=[dict(entry='cli_main')];self.rejected(v)
    def test_boolean_original_exit(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot'][0]['exitCode']=False;self.rejected(v)
    def test_missing_registered_positive(self):
        v=copy.deepcopy(self.value)
        for r in v['groups']['components']['reports']['snapshot']:r['cases'][:]=[c for c in r['cases'] if c['input']['id']!='extract-all']
        self.rejected(v)
    def test_weakened_raw_report(self):
        v=copy.deepcopy(self.value);r=v['groups']['components']['reports']['workspace'][15]['cases'][0];r['expected']=r['actual']={};self.rejected(v)
    def test_source_callback(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['snapshot'][0]['semanticFallbacks']=['cli_main'];self.rejected(v)
    def test_missing_native_call(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['snapshot'][0]['cases'][0]['native']=[];self.rejected(v)
    def test_changed_script_bytes(self):
        v=copy.deepcopy(self.value)
        for report in v['groups']['cli']['reports']['workspace']:
            for row in report['cases']:
                if row['input']['id']=='sign-public-fixture':row['actual']['scriptAfter']=None
        self.rejected(v)
    def test_weakened_production(self):
        v=copy.deepcopy(self.value);r=v['groups']['production']['reports']['snapshot'][1]['cases'][0];r['actual']=r['expected']={};self.rejected(v)
    def test_missing_historical_input(self):
        v=copy.deepcopy(self.value);v['groups']['legacy']['reports']['snapshot'][0]['cases'].pop();self.rejected(v)
    def test_missing_retained_step(self):
        v=copy.deepcopy(self.value);v['updates']['workspace'][0]['cases'].pop();self.rejected(v)
    def test_changed_noop(self):
        v=copy.deepcopy(self.value);v['updates']['snapshot'][0]['cases'][2]['changedPaths']=['a.py'];self.rejected(v)
    def test_weakened_update(self):
        v=copy.deepcopy(self.value);r=v['updates']['snapshot'][0]['cases'][1];r['expected']=r['actual']={};self.rejected(v)
    def test_missing_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases']=[];self.rejected(v)
    def test_ineffective_control(self):
        v=copy.deepcopy(self.value);r=v['controls'][0]['cases'][0];r['actual']=r['expected'];self.rejected(v)
    def test_invalid_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases'][0]['actual']={'error':'compile failure'};self.rejected(v)
    def test_unrelated_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases'][0]['programCopies']={};self.rejected(v)
    def test_wrong_engine(self):
        v=copy.deepcopy(self.value);v['groups']['public']['reports']['workspace'][0]['engineSha256']='other';self.rejected(v)
    def test_wrong_rule(self):
        v=copy.deepcopy(self.value);v['programs'][v['rulePath']]['sha256']='other';self.rejected(v)
    def test_changed_source_text(self):
        v=copy.deepcopy(self.value);v['groups']['components']['source']['text']='{}';self.rejected(v)
    def test_null_native_programs(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['snapshot'][0]['cases'][0]['native'][0]['programInputs']=None;self.rejected(v)


if __name__=='__main__':unittest.main()
