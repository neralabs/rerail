"""Contract receipts must retain stream errors, positives, and their source scope."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from contract_checkpoint import validate


class ContractCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        def load(kind):
            with gzip.open(ROOT/f'gate/data-layer/fixtures/host/data-layer-port/rr-conn-union-{kind}-finalfcce.json.gz','rt') as stream:return json.load(stream)
        cls.components=load('components');cls.production=load('production')

    def test_both_complete_groups_remain_progress(self):
        for value,count in [(self.components,115),(self.production,18)]:
            summary=validate(value)
            self.assertEqual(summary['recordedCasesPerAPI'],count)
            self.assertFalse(summary['admitted'])
            self.assertFalse(summary['fullMigrationAcceptance'])

    def test_source_format_cannot_be_relabelled(self):
        value=copy.deepcopy(self.components);value['sourceFormat']='other'
        with self.assertRaisesRegex(ValueError,'Wrong contract source format'):validate(value)

    def test_missing_signature_warning_is_a_real_parity_failure(self):
        value=copy.deepcopy(self.components)
        row=next(c for r in value['reports']['workspace'] for c in r['cases'] if c['input']['id']=='report-signature-0')
        self.assertTrue(row['expected']['stderr']);row['actual']['stderr']=''
        with self.assertRaisesRegex(ValueError,'complete result differs'):validate(value)

    def test_missing_production_positive_is_rejected(self):
        value=copy.deepcopy(self.production)
        for report in value['reports']['snapshot']:report['cases'][:]=[c for c in report['cases'] if c['input']['id']!='positive']
        with self.assertRaisesRegex(ValueError,'cases omitted or repeated'):validate(value)

    def test_another_engine_cannot_be_used_for_the_second_api(self):
        value=copy.deepcopy(self.production);value['reports']['workspace'][0]['engineSha256']='other'
        with self.assertRaisesRegex(ValueError,'engine or API failed'):validate(value)

    def test_full_admission_cannot_be_claimed_from_a_group(self):
        value=copy.deepcopy(self.components);value['admitted']=True
        with self.assertRaisesRegex(ValueError,'Invalid scanner progress scope'):validate(value)

    def test_original_assertions_cannot_be_skipped(self):
        value=copy.deepcopy(self.production)
        next(iter(value['originals']['workspace']['cases'].values()))[1]['outcome']='skipped'
        with self.assertRaisesRegex(ValueError,'Original scanner assertion failed'):validate(value)


if __name__=='__main__':unittest.main()
