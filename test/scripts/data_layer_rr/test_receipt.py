"""A direct-suite receipt must match the independent original cases and source return values."""
import copy
import importlib.util
from pathlib import Path
import sys
import unittest

directory = Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'
sys.path.insert(0, str(directory))
spec = importlib.util.spec_from_file_location('rr_contract_receipt', directory / 'receipt.py')
receipt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(receipt)


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        self.identity = 'scripts/tests/test_lint_a.py::test_positive'
        source_result = ['the complete original positive return']
        self.baseline = dict(commit=receipt.PIN, pytest_exit_code=0, collected_tests=1, events=[dict(
            test=self.identity, check='a', entry='scan', sourceResult=source_result)])
        self.report = dict(parts=1, part=0, collected=[self.identity], sourcePin=receipt.PIN,
            sourceFiles={'scripts/tests/test_lint_a.py': 'source-hash'}, oracle=False, exitCode=0,
            engineSha256='engine', semanticFallbacks=[], missingResults=[],
            cases={self.identity: [dict(phase=phase, outcome='passed') for phase in ['setup', 'call', 'teardown']]},
            detectorObservations=[dict(test=self.identity, check='a', entry='scan', ordinal=0,
                expectedSourceResult=source_result, actualSourceResult=source_result)],
            calls=[dict(status='passed', diagnostics=[dict(code='POSITIVE')])], casesWithoutNativeCalls=[])
        self.inventory = dict(cases={self.identity: {}})

    def validate(self):
        return receipt.validate(self.baseline, [self.report], self.inventory, 'engine')

    def test_complete_direct_evidence_keeps_source_comparisons(self):
        self.assertEqual(1, self.validate()['losslessApiReturns'])

    def test_an_empty_expected_value_cannot_replace_the_source_positive(self):
        self.report['detectorObservations'][0].update(expectedSourceResult=[], actualSourceResult=[])
        with self.assertRaises(ValueError):
            self.validate()

    def test_a_missing_or_duplicate_source_call_is_rejected(self):
        for observations in [[], self.report['detectorObservations'] * 2]:
            with self.subTest(observations=observations), self.assertRaises(ValueError):
                changed = copy.deepcopy(self.report)
                changed['detectorObservations'] = observations
                receipt.validate(self.baseline, [changed], self.inventory, 'engine')

    def test_skipped_original_assertions_are_rejected(self):
        self.report['cases'][self.identity][1]['outcome'] = 'skipped'
        with self.assertRaises(ValueError):
            self.validate()

    def test_independently_discovered_cases_cannot_be_dropped(self):
        self.inventory['cases']['scripts/tests/test_lint_a.py::test_another_positive'] = {}
        with self.assertRaises(ValueError):
            self.validate()


if __name__ == '__main__':
    unittest.main()
