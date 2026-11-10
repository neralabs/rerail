"""A single original test may contain several independently recorded linter calls."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class OriginalObservationCoverageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-permission-complete-finalec3.json.gz','rt') as stream:
            source=json.loads(json.load(stream)['sources']['original']['text'])
        cls.owned=[r for r in source['events'] if r['check']=='versioned_lifecycle_legacy_surfaces']
        cls.observations=[dict(test=r['test'],entry=r['entry'],ordinal=i,status='passed',
                              expected=r['findings'],actual=r['findings'],actualSourceResult=r['sourceResult']) for i,r in enumerate(cls.owned)]

    def test_all_three_calls_in_one_original_test_are_required(self):
        from recorded_admission import validate_original_observations
        self.assertEqual(3,len(self.owned));self.assertEqual(1,len({r['test'] for r in self.owned}))
        validate_original_observations(self.owned,self.observations)

    def test_omitted_negative_is_rejected(self):
        from recorded_admission import validate_original_observations
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):
            validate_original_observations(self.owned,self.observations[:2])

    def test_repeated_ordinal_is_rejected(self):
        from recorded_admission import validate_original_observations
        observed=copy.deepcopy(self.observations);observed[2]['ordinal']=1
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate_original_observations(self.owned,observed)

    def test_wrong_complete_result_is_rejected(self):
        from recorded_admission import validate_original_observations
        observed=copy.deepcopy(self.observations);observed[0]['actual']=[]
        with self.assertRaisesRegex(ValueError,'complete finding'):validate_original_observations(self.owned,observed)


if __name__=='__main__':unittest.main()
