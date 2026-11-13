"""Discovery progress retains positives, exact updates, and one integrated engine cohort."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from directory_checkpoint import validate


class DirectoryCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-hub-directory-metadata.json.gz','rt')as stream:cls.receipt=json.load(stream)

    def test_complete_progress_preserves_both_apis_and_updates(self):
        self.assertEqual(validate(self.receipt),dict(casesPerAPI=15,discoveryObservations=58,fullMigrationAcceptance=False))

    def test_positive_discovery_cannot_be_dropped(self):
        value=copy.deepcopy(self.receipt);value['native']['snapshot']['cases'].pop(1)
        with self.assertRaisesRegex(ValueError,'input omitted or repeated'):validate(value)

    def test_stale_newline_directory_update_is_rejected(self):
        value=copy.deepcopy(self.receipt);case=value['native']['workspace']['cases'][-1]
        case['actual']['stages'][10]=copy.deepcopy(case['actual']['stages'][9])
        with self.assertRaisesRegex(ValueError,'Discovery result differs'):validate(value)

    def test_both_apis_require_the_same_engine(self):
        value=copy.deepcopy(self.receipt);value['native']['workspace']['engineSha256']='different'
        with self.assertRaisesRegex(ValueError,'Wrong engine cohort'):validate(value)

    def test_discovery_cannot_claim_full_rule_admission(self):
        value=copy.deepcopy(self.receipt);value['fullMigrationAcceptance']=True
        with self.assertRaisesRegex(ValueError,'Invalid discovery scope'):validate(value)


if __name__=='__main__':unittest.main()
