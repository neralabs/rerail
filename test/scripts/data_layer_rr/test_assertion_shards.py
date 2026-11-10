"""Original assertion shards cannot be counted as a complete suite independently."""
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from assertion_shards import merge,merge_production


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

    def test_every_host_transfer_is_retained(self):
        for i,row in enumerate(self.reports):row['hostTransfers']=[dict(part=i)]
        self.assertEqual([dict(part=0),dict(part=1)],merge(self.reports)['hostTransfers'])


class ProductionShardsTests(unittest.TestCase):
    def setUp(self):
        self.reports=[dict(format='production',sourcePin='pin',checks=['backend_type_strings'],part=i,parts=2,
            totalCases=3,baselineSha256='baseline',engineSha256='engine',passed=True,harnessFiles={'h':'hash'},
            cases=[dict(input=dict(id=str(n)),native=dict(a=n,b=n)) for n in range(i,3,2)]) for i in range(2)]

    def test_complete_batch_retains_native_queries(self):
        result=merge_production(self.reports)
        self.assertEqual(3,len(result['cases']))
        self.assertEqual(6,sum(len(c['native']) for c in result['cases']))

    def test_missing_shard_is_rejected(self):
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):merge_production(self.reports[:1])

    def test_duplicate_input_is_rejected(self):
        self.reports[1]['cases'][0]=self.reports[0]['cases'][0]
        with self.assertRaisesRegex(ValueError,'repeated'):merge_production(self.reports)

    def test_different_baseline_is_rejected(self):
        self.reports[1]['baselineSha256']='other'
        with self.assertRaisesRegex(ValueError,'cohort differs'):merge_production(self.reports)


class SeparateModulesTests(unittest.TestCase):
    def setUp(self):
        self.rows=[dict(part=0,parts=1,collected=[name],sourcePin='pin',engineSha256='engine',
            originalRecordingSha256='recording',mode='snapshot',oracle=False,exitCode=0,ruleRoot='rules',
            sourceFiles={name+'.py':'hash'},policySha256={'inspection':'policy'},harnessFiles={'h':'hash'},
            semanticFallbacks=[],missingResults=[],retainedCaseIds=[],testTargets=[name+'.py'],
            cases={name:[dict(phase=p,outcome='passed') for p in ['setup','call','teardown']]},
            calls=[dict(test=name)],detectorObservations=[],assertionTranslations=[],configurationBindings={},
            hostTransfers=[],hostProtocolCalls=[],casesWithoutNativeCalls=[]) for name in ['primary','shared']]

    def test_all_module_cases_and_calls_are_retained(self):
        from assertion_shards import merge_modules
        value=merge_modules(self.rows)
        self.assertEqual({'primary','shared'},set(value['cases']))
        self.assertEqual(2,len(value['calls']));self.assertEqual(2,len(value['sourceFiles']))

    def test_incomplete_module_is_rejected(self):
        from assertion_shards import merge_modules
        self.rows[1]['cases']={}
        with self.assertRaisesRegex(ValueError,'Module assertions omitted'):merge_modules(self.rows)

    def test_duplicate_assertion_is_rejected(self):
        from assertion_shards import merge_modules
        with self.assertRaisesRegex(ValueError,'Module assertion repeated'):merge_modules([self.rows[0],self.rows[0]])

    def test_moving_harness_is_rejected(self):
        from assertion_shards import merge_modules
        self.rows[1]['harnessFiles']['h']='other'
        with self.assertRaisesRegex(ValueError,'Module input differs'):merge_modules(self.rows)


if __name__=='__main__':unittest.main()
