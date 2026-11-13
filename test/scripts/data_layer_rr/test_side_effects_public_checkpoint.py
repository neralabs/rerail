"""Public scanner receipts preserve all original assertions, positives, and exact failures."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from side_effects_public_checkpoint import validate


class SideEffectsPublicCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-side-effects-public-current.json.gz','rt')as stream:cls.receipt=json.load(stream)

    def test_complete_original_and_scanner_groups_pass_both_apis(self):
        self.assertEqual(validate(self.receipt),dict(originalAssertionsPerAPI=7,scannerCasesPerAPI=35,fullMigrationAcceptance=False))

    def test_a_dropped_original_assertion_is_rejected(self):
        value=copy.deepcopy(self.receipt);value['originals']['workspace']['cases'].pop(next(iter(value['originals']['workspace']['cases'])))
        with self.assertRaisesRegex(ValueError,'Original observations differ'):validate(value)

    def test_an_alias_positive_cannot_be_dropped(self):
        value=copy.deepcopy(self.receipt);row=next(r for r in value['scanners']['snapshot']['cases']if r['input']['id']=='file_links');row['actual']['value'].pop()
        with self.assertRaisesRegex(ValueError,'Complete scanner result differs'):validate(value)

    def test_decode_bytes_and_positions_cannot_be_approximated(self):
        value=copy.deepcopy(self.receipt);row=next(r for r in value['scanners']['workspace']['cases']if r['input']['id']=='utf8-3');row['actual']['error']['args']['items'][3]=1
        with self.assertRaisesRegex(ValueError,'Complete scanner result differs'):validate(value)

    def test_public_scanning_cannot_claim_full_rule_admission(self):
        value=copy.deepcopy(self.receipt);value['fullMigrationAcceptance']=True
        with self.assertRaisesRegex(ValueError,'not full rule admission'):validate(value)


if __name__=='__main__':unittest.main()
