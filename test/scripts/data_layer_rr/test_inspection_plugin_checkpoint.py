"""Progress reports must account for every primary and shared assertion."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from inspection_plugin_checkpoint import assertion_coverage,PENDING


def phases(outcome='passed'):
    return [dict(phase=name,outcome=outcome if name=='call' else 'passed') for name in ['setup','call','teardown']]


class InspectionPluginCheckpointTests(unittest.TestCase):
    def example(self):
        wanted={'primary::positive','shared::positive',PENDING}
        reports=[dict(cases={'primary::positive':phases()}),
                 dict(cases={'shared::positive':phases(),PENDING:phases('failed')})]
        return wanted,reports

    def test_separate_modules_cover_the_complete_inventory(self):
        wanted,reports=self.example()
        self.assertEqual(assertion_coverage(wanted,reports),
            dict(required=3,passed=2,failed=1,pending=[PENDING]))

    def test_missing_primary_module_is_rejected(self):
        wanted,reports=self.example()
        with self.assertRaisesRegex(ValueError,'omitted'):assertion_coverage(wanted,reports[1:])

    def test_excluding_pending_shared_test_is_rejected(self):
        wanted,reports=self.example();del reports[1]['cases'][PENDING]
        with self.assertRaisesRegex(ValueError,'omitted'):assertion_coverage(wanted,reports)

    def test_duplicate_case_is_rejected(self):
        wanted,reports=self.example();reports.append(reports[0])
        with self.assertRaisesRegex(ValueError,'Duplicate'):assertion_coverage(wanted,reports)

    def test_extra_failure_is_rejected(self):
        wanted,reports=self.example();reports[0]['cases']['primary::positive']=phases('failed')
        with self.assertRaisesRegex(ValueError,'Unexpected'):assertion_coverage(wanted,reports)

    def test_duplicate_phase_cannot_hide_a_failed_assertion(self):
        wanted,reports=self.example();reports[0]['cases']['primary::positive'].append(dict(phase='call',outcome='passed'))
        with self.assertRaisesRegex(ValueError,'phase'):assertion_coverage(wanted,reports)
