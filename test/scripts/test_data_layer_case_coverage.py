"""A passing positive/clean total must not hide a dropped original call."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts/dsl'))
from data_layer_case_coverage import audit_calls


class CaseCoverage(unittest.TestCase):
    def setUp(self):
        self.events = [dict(check='rule', test='test.py::positive', call=n,
                            upstream_passed=True, findings=[{'code': str(n)}]) for n in (1, 2)]
        self.rows = [dict(check='rule', test='test.py::positive', call=n,
                         execution='native', expected=1, actual=1, positive=True,
                         falsePositives=[], falseNegatives=[], engineFailures=[],
                         statelessFailures=[], executionPathDifferences=[]) for n in (1, 2)]

    def test_checks_every_call_in_a_test_with_multiple_positives(self):
        self.assertEqual(audit_calls(self.events, self.rows), {'test.py::positive': 2})

    def test_rejects_dropped_positive_despite_other_positive_coverage(self):
        with self.assertRaisesRegex(ValueError, 'Missing original call'):
            audit_calls(self.events, self.rows[:1])

    def test_rejects_duplicate_replay_that_preserves_the_total(self):
        with self.assertRaisesRegex(ValueError, 'Duplicate replay call'):
            audit_calls(self.events, [self.rows[0], self.rows[0]])

    def test_rejects_wrong_findings_even_when_counts_match(self):
        rows = copy.deepcopy(self.rows)
        rows[0]['falseNegatives'] = ['wrong symbol']
        with self.assertRaisesRegex(ValueError, 'Original call differs'):
            audit_calls(self.events, rows)


if __name__ == '__main__':
    unittest.main()
