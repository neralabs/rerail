"""Checkpoint audits must reject a lost positive, changed DTO, or missing native execution."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from steps_checkpoint import assemble
from migration import PIN


class StepsCheckpointTests(unittest.TestCase):
    def setUp(self):
        call = dict(test='positive', entry='check', arguments={'source':'value'},
                    **{'return':[dict(code='X',line=2,message='full positive')]}, stdout='', stderr='')
        self.source = dict(sourcePin=PIN, pytest_exit_code=0, collected=['negative','positive','unported'],
                           calls=[dict(call,test='negative',**{'return':[]}),call,
                                  dict(call,test='unported',entry='missing')])
        self.report = dict(sourcePin=PIN, exitCode=0, semanticFallbacks=[], hostProtocolCalls=[], missingResults=[],
            collected=self.source['collected'], part=0, parts=1, selectedEntries='check', engineSha256='engine',
            harnessFiles={'contract':'hash'}, selectedCases=['negative','positive'],
            cases={t:[dict(phase=p,outcome='passed') for p in ['setup','call','teardown']]
                   for t in ['negative','positive']}, calls=copy.deepcopy(self.source['calls'][:2]),
            nativeCalls=[dict(status='passed',programInputs=['program'])])

    def test_complete_selected_observations_keep_the_unported_case_explicit(self):
        result = assemble(self.source, [self.report])
        self.assertFalse(result['fullMigrationAcceptance'])
        self.assertEqual(['unported'], result['untestedCases'])
        self.assertEqual(2, result['selectedAssertions'])
        self.assertEqual('full positive', result['observations'][1]['actual']['return'][0]['message'])

    def test_lost_positive_or_its_observation_is_rejected(self):
        for change in ['selected', 'phase', 'observation', 'shard']:
            report = copy.deepcopy(self.report)
            if change == 'selected':report['selectedCases'].remove('positive')
            if change == 'phase':del report['cases']['positive']
            if change == 'observation':report['calls'].pop()
            if change == 'shard':report['parts']=2
            with self.subTest(change=change), self.assertRaises((AssertionError,ValueError)):
                assemble(self.source,[report])

    def test_changed_positive_fields_and_duplicate_shards_are_rejected(self):
        for field in ['line','code','message']:
            report = copy.deepcopy(self.report)
            report['calls'][1]['return'][0][field] = 'changed'
            with self.subTest(field=field), self.assertRaises(AssertionError):
                assemble(self.source,[report])
        with self.assertRaises(ValueError):
            assemble(self.source,[self.report,self.report])

    def test_failed_native_execution_and_python_fallback_are_rejected(self):
        for field in ['nativeCalls','noNativeCalls','semanticFallbacks','missingResults']:
            report = copy.deepcopy(self.report)
            if field == 'noNativeCalls':report['nativeCalls'] = []
            else:report[field] = [dict(status='failed')] if field == 'nativeCalls' else ['untranslated']
            with self.subTest(field=field), self.assertRaises(ValueError):
                assemble(self.source,[report])

    def test_pytest_no_tests_exit_is_valid_only_for_an_empty_selected_shard(self):
        report = copy.deepcopy(self.report)
        report.update(exitCode=5, selectedEntries='missing', selectedCases=['unported'])
        with self.assertRaises(ValueError):
            assemble(self.source, [report])
        report.update(selectedEntries='another_entry', selectedCases=[], cases={}, calls=[], nativeCalls=[])
        result = assemble(self.source, [report])
        self.assertEqual(0, result['selectedAssertions'])
        self.assertEqual(self.source['collected'], result['untestedCases'])
        report.update(selectedEntries='check')
        with self.assertRaises((AssertionError, ValueError)):
            assemble(self.source, [report])
