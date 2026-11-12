"""Require the entire unresolved-reference contract before marking its migration complete."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from unresolved_admission import validate


class Admission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-unresolved-complete-finalfcce.json.gz','rt')as stream:cls.value=json.load(stream)
    def rejected(self,v):
        with self.assertRaises(ValueError):validate(v)
    def test_complete(self):
        s=validate(self.value);self.assertTrue(s['admitted']);self.assertTrue(s['fullRuleMigrationAcceptance']);self.assertFalse(s['fullMigrationAcceptance'])
        self.assertEqual(s['originalCasesPerAPI'],119);self.assertEqual(s['originalObservationsPerAPI'],63);self.assertEqual(s['originalPositiveObservationsPerAPI'],37)
        self.assertEqual(s['casesPerAPI'],dict(components=50,public=206,cli=32,production=25,legacy=74));self.assertEqual(s['controls'],30);self.assertEqual(s['persistentStepsPerAPI'],19)
        self.assertEqual(s['sharedHelper']['casesPerAPI'],117)
    def test_missing_original(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot'][0]['cases'].popitem();self.rejected(v)
    def test_missing_original_positive(self):
        v=copy.deepcopy(self.value)
        for report in v['originals']['workspace']:
            selected=next((i for i,r in enumerate(report['detectorObservations'])if r.get('actualSourceResult')),None)
            if selected is not None:report['detectorObservations'].pop(selected);break
        self.rejected(v)
    def test_changed_original_dto(self):
        v=copy.deepcopy(self.value)
        for report in v['originals']['snapshot']:
            if report['detectorObservations']:report['detectorObservations'][0]['actualSourceResult']={};break
        self.rejected(v)
    def test_skipped_original(self):
        v=copy.deepcopy(self.value);next(iter(v['originals']['workspace'][0]['cases'].values()))[1]['outcome']='skipped';self.rejected(v)
    def test_unbound_constants(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot'][0]['configurationBindings']['unresolved_forward_refs']['names']=[];self.rejected(v)
    def test_host_semantics(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot'][0]['hostProtocolCalls']=[dict(entry='cli_main')];self.rejected(v)
    def test_unapproved_declaration(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot'][0]['passiveDeclarationCalls']=[dict(declaration='scan')];self.rejected(v)
    def test_missing_declaration_record(self):
        v=copy.deepcopy(self.value)
        for r in v['originals']['workspace']:r['passiveDeclarationCalls']=[]
        self.rejected(v)
    def test_missing_public_positive(self):
        v=copy.deepcopy(self.value);v['groups']['public']['reports']['workspace'][0]['cases'].pop();self.rejected(v)
    def test_weakened_public_result(self):
        v=copy.deepcopy(self.value);r=v['groups']['public']['reports']['snapshot'][0]['cases'][0];r['expected']=r['actual']={};self.rejected(v)
    def test_source_callback(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['workspace'][0]['semanticFallbacks']=['cli_main'];self.rejected(v)
    def test_missing_native_call(self):
        v=copy.deepcopy(self.value);v['groups']['public']['reports']['snapshot'][0]['cases'][0]['native']=[];self.rejected(v)
    def test_missing_historical_case(self):
        v=copy.deepcopy(self.value);v['groups']['legacy']['reports']['snapshot'][0]['cases'].pop();self.rejected(v)
    def test_weakened_production(self):
        v=copy.deepcopy(self.value);r=v['groups']['production']['reports']['workspace'][1]['cases'][0];r['actual']=r['expected']={};self.rejected(v)
    def test_missing_update(self):
        v=copy.deepcopy(self.value);v['updates']['snapshot'][0]['cases'].pop();self.rejected(v)
    def test_changed_noop(self):
        v=copy.deepcopy(self.value);v['updates']['workspace'][0]['cases'][2]['changedPaths']=['api/a.py'];self.rejected(v)
    def test_weakened_retained_result(self):
        v=copy.deepcopy(self.value);r=v['updates']['workspace'][1]['cases'][1];r['actual']=r['expected']={};self.rejected(v)
    def test_missing_control(self):
        v=copy.deepcopy(self.value);v['controls'].pop();self.rejected(v)
    def test_ineffective_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['actual']=v['controls'][0]['expected'];self.rejected(v)
    def test_invalid_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['actual']=dict(error='compile failure');self.rejected(v)
    def test_unrelated_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['programCopies']={};self.rejected(v)
    def test_changed_shared_helper(self):
        v=copy.deepcopy(self.value);v['sharedHelper']['reports']['workspace'][0]['cases'].pop();self.rejected(v)
    def test_wrong_engine(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['snapshot'][0]['engineSha256']='other';self.rejected(v)
    def test_wrong_main_program(self):
        v=copy.deepcopy(self.value);v['programs'][v['rulePath']]['sha256']='other';self.rejected(v)
    def test_changed_source_bytes(self):
        v=copy.deepcopy(self.value);v['originalSource']['text']='{}';self.rejected(v)
    def test_null_consumed_programs(self):
        v=copy.deepcopy(self.value);v['groups']['public']['reports']['snapshot'][0]['cases'][0]['native'][0]['programInputs']=None;self.rejected(v)


if __name__=='__main__':unittest.main()
