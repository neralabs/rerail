"""Async-pattern admission must retain every assertion and complete native evidence."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from async_pattern_admission import validate


class Admission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-async-pattern-complete-finalfcce.json.gz','rt') as f:cls.value=json.load(f)

    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)

    def test_complete(self):
        s=validate(self.value)
        self.assertTrue(s['admitted'])
        self.assertTrue(s['fullRuleMigrationAcceptance'])
        self.assertFalse(s['fullMigrationAcceptance'])
        self.assertEqual(s['originalCasesPerAPI'],11)
        self.assertEqual(s['originalPositiveObservationsPerAPI'],8)
        self.assertEqual(s['casesPerAPI'],dict(helpers=137,components=162,files=41,cli=40,production=19,legacy=38))
        self.assertEqual(s['supplementalCasesSnapshot'],dict(api=158,report=136))
        self.assertEqual(s['persistentStepsPerAPI'],20)
        self.assertEqual(s['controls'],48)

    def test_missing_original_positive(self):
        v=copy.deepcopy(self.value);v['originals']['workspace']['detectorObservations'].pop();self.rejected(v)

    def test_missing_original_assertion(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot']['cases'].popitem();self.rejected(v)

    def test_missing_legacy_case(self):
        v=copy.deepcopy(self.value);v['groups']['legacy']['reports']['snapshot'][0]['cases'].pop();self.rejected(v)

    def test_missing_collector_positive(self):
        v=copy.deepcopy(self.value)
        for r in v['groups']['components']['reports']['snapshot']:r['cases'][:]=[c for c in r['cases'] if c['input']['id']!='collector-ignores-prefilter']
        self.rejected(v)

    def test_missing_direct_file_case(self):
        v=copy.deepcopy(self.value)
        for r in v['groups']['files']['reports']['workspace']:r['cases'][:]=[c for c in r['cases'] if c['input']['id']!='lint-file-9']
        self.rejected(v)

    def test_changed_cli_write(self):
        v=copy.deepcopy(self.value)
        for r in v['groups']['cli']['reports']['snapshot']:
            for c in r['cases']:
                if c['input']['id']=='update':c['actual']['baselineAfter']=None
        self.rejected(v)

    def test_changed_full_helper_value(self):
        v=copy.deepcopy(self.value);v['groups']['helpers']['reports']['workspace'][0]['cases'][0]['actual']={};self.rejected(v)

    def test_hidden_native_work(self):
        v=copy.deepcopy(self.value);v['groups']['files']['reports']['snapshot'][0]['cases'][0]['native']=[];self.rejected(v)

    def test_source_semantic_fallback(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['snapshot'][0]['semanticFallbacks']=['cli_main'];self.rejected(v)

    def test_missing_update_step(self):
        v=copy.deepcopy(self.value);v['updates']['snapshot'][0]['cases'].pop();self.rejected(v)

    def test_changed_noop(self):
        v=copy.deepcopy(self.value);v['updates']['workspace'][0]['cases'][2]['changedPaths']=['api/unrelated.py'];self.rejected(v)

    def test_weakened_update(self):
        v=copy.deepcopy(self.value);c=v['updates']['snapshot'][0]['cases'][1];c['actual']=c['expected']={};self.rejected(v)

    def test_missing_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases'].pop();self.rejected(v)

    def test_ineffective_control(self):
        v=copy.deepcopy(self.value);c=v['controls'][0]['cases'][0];c['actual']=c['expected'];self.rejected(v)

    def test_invalid_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases'][0]['actual']={'error':'bad native program'};self.rejected(v)

    def test_unrelated_mutation(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases'][0]['programCopies']={};self.rejected(v)

    def test_missing_alias_scope_control(self):
        v=copy.deepcopy(self.value)
        for r in v['controls']:r['cases'][:]=[c for c in r['cases'] if c['control']!='alias-scope']
        self.rejected(v)

    def test_mixed_engine(self):
        v=copy.deepcopy(self.value);v['updates']['workspace'][0]['engineSha256']='different';self.rejected(v)

    def test_mixed_rule(self):
        v=copy.deepcopy(self.value);v['programs'][v['rulePath']]['sha256']='different';self.rejected(v)

    def test_mixed_harness(self):
        v=copy.deepcopy(self.value);r=v['groups']['helpers']['reports']['snapshot'][0];r['harnessFiles']['scripts/dsl/data_layer_rr/suite.py']='different';self.rejected(v)


if __name__=='__main__':unittest.main()
