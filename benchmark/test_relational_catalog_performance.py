"""Execution accounting must not hide physical work from the catalog gate."""
import copy
import importlib.util
from pathlib import Path
import unittest

_spec = importlib.util.spec_from_file_location(
    'catalog_gate', Path(__file__).with_name('relational-catalog-performance.py'))
_gate = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_gate)


class PhysicalAccountingTest(unittest.TestCase):
    def setUp(self):
        self.budget = {'fixtures': [], 'budgets': [{
            'policy': 'p', 'clause': 'c', 'grammar': 0, 'maxVm': 100,
            'nativeOutputSha256': 'same', 'matchingStatements': 1,
            'framePublications': 0}]}
        self.execution = {'passed': True, 'vm': 90, 'nativeOutputSha256': 'same',
                          'matchingStatements': 3, 'framePublications': 0,
                          'finalStatement': {'matchingStatements': 1, 'vmSteps': 70},
                          'stageExecution': {'stageExecutions': 2,
                              'vmSteps': 20,
                              'expectedMatchingStatements': 3,
                              'statements': [{'matchingStatements': 1, 'vmSteps': 10},
                                             {'matchingStatements': 1, 'vmSteps': 10}]}}

    def errors(self, execution):
        report = {'fixtures': [], 'clauses': [{'policy': 'p', 'clause': 'c',
            'grammars': [{'grammar': 0, 'result': {
                'admitted': True, 'execution': execution}}]}]}
        return _gate.check(report, self.budget)

    def test_accounted_cuts_retain_total_vm_ceiling(self):
        self.assertFalse(self.errors(self.execution))
        self.execution['vm'] = 101
        self.execution['finalStatement']['vmSteps'] = 81
        self.assertTrue(self.errors(self.execution))

    def test_stage_vm_must_reconcile_with_the_final_total(self):
        for field, value in [('vm', 70), ('vm', 89)]:
            execution = copy.deepcopy(self.execution)
            execution[field] = value
            self.assertTrue(self.errors(execution))
        for target in ['vmSteps', 'statements']:
            execution = copy.deepcopy(self.execution)
            if target == 'statements':
                execution['stageExecution'][target][0].pop('vmSteps')
            else:
                execution['stageExecution'][target] = 0
            self.assertTrue(self.errors(execution))

    def test_missing_or_extra_executions_fail(self):
        for count in [1, 2, 4, 101]:
            execution = copy.deepcopy(self.execution)
            execution['matchingStatements'] = count
            self.assertTrue(self.errors(execution))
        self.execution.pop('stageExecution')
        self.assertTrue(self.errors(self.execution))

    def test_unreported_or_batched_stage_executions_fail(self):
        for count in [0, 2]:
            execution = copy.deepcopy(self.execution)
            execution['stageExecution']['statements'][0]['matchingStatements'] = count
            self.assertTrue(self.errors(execution))

    def test_final_matching_count_stays_pinned(self):
        self.execution['finalStatement']['matchingStatements'] = 2
        self.execution['matchingStatements'] = 4
        self.execution['stageExecution']['expectedMatchingStatements'] = 4
        self.assertTrue(self.errors(self.execution))

    def test_fully_reported_job_still_has_a_hard_cap(self):
        self.execution['stageExecution']['statements'] = [{'matchingStatements': 1}] * 100
        self.execution['stageExecution']['stageExecutions'] = 100
        self.execution['stageExecution']['expectedMatchingStatements'] = 101
        self.execution['matchingStatements'] = 101
        self.assertTrue(self.errors(self.execution))


if __name__ == '__main__':
    unittest.main()
