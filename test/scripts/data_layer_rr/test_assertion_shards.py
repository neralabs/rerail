"""Original assertion shards cannot be counted as a complete suite independently."""
import copy
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from assertion_shards import merge


class AssertionShardsTests(unittest.TestCase):
    def setUp(self):
        collected=['a','b','c','d','e']
        self.reports=[dict(part=part,parts=2,collected=collected,sourcePin='pin',engineSha256='engine',sourceFiles={},
            policySha256={},originalRecordingSha256='recording',mode='snapshot',oracle=False,exitCode=0,
            semanticFallbacks=[],missingResults=[],cases={name:[dict(phase=p,outcome='passed') for p in ['setup','call','teardown']]
                for name in collected[part::2]},calls=[dict(part=part)],detectorObservations=[dict(part=part)],
            harnessFiles={'h':'hash'},assertionTranslations=[],hostProtocolCalls=[],casesWithoutNativeCalls=[],configurationBindings={}) for part in range(2)]

    def test_complete_coverage_keeps_every_call(self):
        report=merge(self.reports)
        self.assertEqual(set(report['cases']),set(self.reports[0]['collected']))
        self.assertEqual(2,len(report['calls']));self.assertEqual(2,len(report['detectorObservations']))

    def test_missing_shard_is_rejected(self):
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):merge(self.reports[:1])

    def test_missing_case_is_rejected(self):
        self.reports[0]['cases'].pop('a')
        with self.assertRaisesRegex(ValueError,'case coverage'):merge(self.reports)

    def test_mixed_engine_is_rejected(self):
        self.reports[1]['engineSha256']='other'
        with self.assertRaisesRegex(ValueError,'cohort differs'):merge(self.reports)


if __name__=='__main__':unittest.main()
