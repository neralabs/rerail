"""Preserve supplied-tree and complete raw-fact contracts across both native interfaces."""
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from unresolved_components_contract import cases
from validate_supplied_facts_checkpoint import equal


class Components(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-unresolved-components-native.json.gz','rt') as stream:cls.proof=json.load(stream)

    def test_complete_source_recorded_inputs(self):
        source=self.proof['source'];self.assertTrue(source['passed'])
        self.assertEqual([r['input']for r in source['cases']],cases(None))
        self.assertEqual(len(source['cases']),50)

    def test_both_interfaces_preserve_all_results(self):
        source={r['input']['id']:r for r in self.proof['source']['cases']}
        for mode in ['snapshot','workspace']:
            rows=[]
            for shard in self.proof['reports'][mode]:
                self.assertTrue(shard['passed']);self.assertEqual(shard['semanticFallbacks'],[])
                self.assertEqual(shard['executionMode'],mode);rows.extend(shard['cases'])
            self.assertEqual(len(rows),len(source));self.assertEqual({r['input']['id']for r in rows},set(source))
            for row in rows:
                with self.subTest(mode=mode,case=row['input']['id']):
                    self.assertTrue(equal(row['expected'],source[row['input']['id']]['expected']))
                    self.assertTrue(equal(row['actual'],row['expected']))

    def test_required_adversarial_inputs_remain(self):
        ids={r['input']['id']for r in self.proof['source']['cases']}
        self.assertTrue({'source-missing-tree','source-source-clean','source-different-tree','source-syntax-in-source',
                         'state-positive','extract-unicode','extract-model_fields','report-order','report-fallback-order'}<=ids)

    def test_checkpoint_does_not_admit_rule(self):
        self.assertFalse(self.proof['fullMigrationAcceptance']);self.assertFalse(self.proof['fullRuleMigrationAcceptance'])
        self.assertIn('native CLI',self.proof['remaining'])


if __name__=='__main__':unittest.main()
