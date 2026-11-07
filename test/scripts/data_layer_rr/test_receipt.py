"""A direct-suite receipt must match the independent original cases and source return values."""
import copy
import importlib.util
from pathlib import Path
import sys
import unittest
from enum import Enum

directory = Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'
sys.path.insert(0, str(directory))
from dto import canonical
spec = importlib.util.spec_from_file_location('rr_contract_receipt', directory / 'receipt.py')
receipt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(receipt)
import mutation


class ReceiptTests(unittest.TestCase):
    def test_ownership_mutation_keeps_inventory_and_disables_every_finding(self):
        self.assertTrue(mutation.auxiliary_output('plan_scope_ownership',dict(code='PSO_FILE')))
        for i in range(14):self.assertFalse(mutation.auxiliary_output('plan_scope_ownership',dict(code=f'PSO{i:03}')))
        source='Selected(p) :- inventory(p).\n'+''.join(f'Ownership{i}(p, m, s, e, r, w) :- issue{i}(p).\n' for i in range(14))
        disabled=mutation.disable_policy('plan_scope_ownership',source)
        self.assertIn('Selected(p) :- inventory(p).',disabled)
        self.assertEqual(14,disabled.count(':- 1 = 0,'))
        with self.assertRaises(ValueError):mutation.disable_policy('plan_scope_ownership','')

    def test_boundary_mutation_preserves_counts_and_cannot_hide_bridge_findings(self):
        self.assertTrue(mutation.auxiliary_output('async_sync_boundary', dict(code='BS_COUNT')))
        self.assertFalse(mutation.auxiliary_output('async_sync_boundary', dict(code='SYNC_WRAPS_ASYNC')))
        self.assertFalse(mutation.auxiliary_output('async_sync_boundary', dict(code='ASYNC_WRAPS_SYNC')))
        self.assertFalse(mutation.auxiliary_output('async_sync_boundary', dict(code='EXECUTOR_ESCAPE')))
        source = 'Count(a, n) :- input(n).\n' + ''.join('bs_detect(c, "' + c + '", f, a, t) :- candidate(c).\n'
            for c in ['SYNC_WRAPS_ASYNC', 'ASYNC_WRAPS_SYNC', 'EXECUTOR_ESCAPE'])
        disabled = mutation.disable_policy('async_sync_boundary', source)
        self.assertIn('Count(a, n) :- input(n).', disabled)
        self.assertEqual(3, disabled.count(':- 1 = 0,'))
        with self.assertRaises(ValueError):
            mutation.disable_policy('async_sync_boundary', 'Count(a, n) :- input(n).')

    def test_positive_controls_keep_only_known_auxiliary_outputs(self):
        self.assertTrue(mutation.auxiliary_output('backend_coupling', dict(code='BC_CONFIG')))
        self.assertTrue(mutation.auxiliary_output('backend_coupling', dict(code='BC_CHECK')))
        self.assertFalse(mutation.auxiliary_output('backend_coupling', dict(code='direct_backend_import')))
        self.assertFalse(mutation.auxiliary_output('backend_coupling', dict(code='sql_in_nonsql')))
        self.assertFalse(mutation.auxiliary_output('unknown', dict(code='BC_CONFIG')))

    def test_backend_mutation_disables_detection_and_preserves_configuration(self):
        source = 'bc_config("BACKEND_PREFIXES", 0, "impl.sqlite").\nCoupling(st, code, message, i, depth) :- bc_issue(st, code, message), visit_index(st, i), ast_depth(st, depth).\n'
        disabled = mutation.disable_policy('backend_coupling', source)
        self.assertIn('bc_config("BACKEND_PREFIXES", 0, "impl.sqlite").', disabled)
        self.assertIn(':- 1 = 0, bc_issue', disabled)
        with self.assertRaises(ValueError):
            mutation.disable_policy('backend_coupling', '')

    def test_cache_mutation_cannot_hide_real_findings_as_helpers(self):
        self.assertTrue(mutation.auxiliary_output('connection_caches', dict(code='CC_CONFIG')))
        self.assertTrue(mutation.auxiliary_output('connection_caches', dict(code='CC_PATTERN')))
        self.assertFalse(mutation.auxiliary_output('connection_caches', dict(code='CC001')))
        source = 'cc_pattern(0, "regex", "description").\nCache(at, i, snippet, description) :- cc_hit(at, i, snippet, description).\n'
        disabled = mutation.disable_policy('connection_caches', source)
        self.assertIn('cc_pattern(0, "regex", "description").', disabled)
        self.assertIn(':- 1 = 0, cc_hit', disabled)

    def test_binary_exception_arguments_keep_every_byte(self):
        self.assertEqual({'kind': 'bytes', 'hex': '00fffe'}, canonical(bytes([0, 255, 254])))

    def test_enum_dto_keeps_the_class_name_member_and_value(self):
        class Category(Enum):
            POSITIVE = 'the source category'
        value = canonical(Category.POSITIVE)
        self.assertEqual('enum', value['kind'])
        self.assertTrue(value['type'].endswith('.Category'))
        self.assertEqual('POSITIVE', value['name'])
        self.assertEqual('the source category', value['value'])

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
