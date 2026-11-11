"""Admission must retain the complete original and supplemental source contracts."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from conn_union_admission import validate


class Admission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-conn-union-complete-finalfcce.json.gz','rt') as stream:cls.receipt=json.load(stream)

    def changed(self):return copy.deepcopy(self.receipt)

    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)

    def test_complete_receipt(self):
        summary=validate(self.receipt)
        self.assertTrue(summary['admitted'])
        self.assertEqual(summary['originalCasesPerAPI'],5)
        self.assertEqual(summary['originalPositiveObservationsPerAPI'],2)
        self.assertEqual(summary['casesPerAPI'],dict(scanner=64,components=118,production=18,cli=36,legacy=15))
        self.assertEqual(summary['persistentStepsPerAPI'],22)
        self.assertEqual(summary['controls'],24)

    def test_omitted_original_positive(self):
        value=self.changed();value['originals']['snapshot']['detectorObservations'].pop(0);self.rejected(value)

    def test_omitted_component(self):
        value=self.changed();value['reports']['components']['workspace'][0]['cases'].pop();self.rejected(value)

    def test_mixed_engine(self):
        value=self.changed();value['reports']['cli']['snapshot'][0]['engineSha256']='changed';self.rejected(value)

    def test_changed_stream(self):
        value=self.changed();value['reports']['production']['snapshot'][0]['cases'][0]['actual']['stderr']='omitted warning';self.rejected(value)

    def test_missing_native_execution(self):
        value=self.changed();value['reports']['components']['snapshot'][0]['cases'][0]['native']=[];self.rejected(value)

    def test_missing_update(self):
        value=self.changed();value['updates']['workspace'][0]['cases'].pop();self.rejected(value)

    def test_forged_unchanged_step(self):
        value=self.changed();value['updates']['snapshot'][0]['cases'][2]['changedPaths']=['impl/a.py'];self.rejected(value)

    def test_missing_control(self):
        value=self.changed();value['controls'][0]['cases'].pop();self.rejected(value)

    def test_ineffective_control(self):
        value=self.changed();row=value['controls'][0]['cases'][0];row['actual']=row['expected'];self.rejected(value)

    def test_invalid_control(self):
        value=self.changed();value['controls'][0]['cases'][0]['actual']={'error':'compile failure'};self.rejected(value)

    def test_changed_program(self):
        value=self.changed();next(p for p in value['programs'].values() if p['text'])['text']+='\n';self.rejected(value)

    def test_changed_source_snapshot(self):
        value=self.changed();value['sources']['cli']['text']+='\n';self.rejected(value)

    def test_omitted_legacy_input(self):
        value=self.changed();value['reports']['legacy']['snapshot'][0]['cases'].pop();self.rejected(value)


if __name__=='__main__':unittest.main()
