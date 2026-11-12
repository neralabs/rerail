"""Retain exact CLI streams, callbacks and raised-versus-returned status comparisons."""
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from unresolved_cli_contract import cases
from validate_supplied_facts_checkpoint import equal


class Cli(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-unresolved-cli-native.json.gz','rt') as stream:cls.proof=json.load(stream)

    def test_complete_source_inputs(self):
        self.assertTrue(self.proof['source']['passed'])
        self.assertEqual([r['input']for r in self.proof['source']['cases']],cases(None))
        self.assertEqual(len(self.proof['source']['cases']),32)

    def test_complete_output_and_status_in_both_interfaces(self):
        expected={r['input']['id']:r['expected']for r in self.proof['source']['cases']}
        for mode in ['snapshot','workspace']:
            rows=[]
            for shard in self.proof['reports'][mode]:
                self.assertTrue(shard['passed']);self.assertEqual(shard['semanticFallbacks'],[])
                self.assertEqual(shard['executionMode'],mode);rows.extend(shard['cases'])
            self.assertEqual(len(rows),len(expected));self.assertEqual({r['input']['id']for r in rows},set(expected))
            for row in rows:
                with self.subTest(mode=mode,case=row['input']['id']):
                    self.assertTrue(equal(row['actual'],expected[row['input']['id']]))
                    self.assertTrue(equal(row['expected'],expected[row['input']['id']]))
                    self.assertEqual(set(row['actual']),{'value','events','stdout','stderr'})

    def test_no_admission_claim(self):
        self.assertFalse(self.proof['fullMigrationAcceptance']);self.assertFalse(self.proof['fullRuleMigrationAcceptance'])


if __name__=='__main__':unittest.main()
