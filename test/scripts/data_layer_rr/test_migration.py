"""The format migration must retain every authored rule and every assertion."""
import importlib.util
import tempfile
import unittest
import sys
from pathlib import Path


def module():
    path = Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr/migration.py'
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location('rr_migration', path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


class MigrationTests(unittest.TestCase):
    def test_fresh_arguments_distinguish_an_empty_set_from_an_absent_override(self):
        module()
        from fresh import arguments
        value=arguments(dict(explicit={'frozenset':[]},missing=None,names={'frozenset':['A','A','B']}),Path('/fixture'))
        self.assertEqual(value,dict(explicit=frozenset(),missing=None,names=frozenset({'A','B'})))

    def test_fresh_shards_keep_global_oracle_indices_and_cover_every_case_once(self):
        module()
        from fresh import selected_cases
        cases = [{'id': str(i)} for i in range(7)]
        shards = [selected_cases(cases, part, 3) for part in range(3)]
        merged = sorted(row for shard in shards for row in shard)
        self.assertEqual(list(enumerate(cases)), merged)
        self.assertEqual([0, 3, 6], [index for index, _ in shards[0]])

    def test_fresh_shards_reject_an_invalid_partition(self):
        module()
        from fresh import selected_cases
        for part, parts in [(-1, 2), (2, 2), (0, 0), (0, 3)]:
            with self.subTest(part=part, parts=parts), self.assertRaises(ValueError):
                selected_cases([{'id': 'a'}, {'id': 'b'}], part, parts)

    def test_fresh_arguments_keep_a_path_keyword_dictionary(self):
        module()
        from fresh import arguments
        self.assertEqual(arguments({'path': {'path': 'impl/a.py'}}, Path('/fixture')),
                         {'path': Path('/fixture/impl/a.py')})

    def test_reporter_file_groups_preserve_every_in_file_value_and_order(self):
        module()
        from reporting import reporter_file_groups
        first = dict(path='a.py', line=2, message='second')
        second = dict(path='a.py', line=1, message='first')
        other = dict(path='b.py', line=1, message='other')
        self.assertEqual(reporter_file_groups([other, first, second, second]),
                         reporter_file_groups([first, second, second, other]))
        self.assertNotEqual(reporter_file_groups([first, second]), reporter_file_groups([second, first]))
        self.assertNotEqual(reporter_file_groups([first, second, second]), reporter_file_groups([first, second]))

    def test_source_dto_keeps_snippets_tuple_shape_and_duplicates(self):
        m = module()
        from dto import canonical
        original = [(Path('/tmp/original/a.py'), 1, 'full snippet', 'label')] * 2
        actual = [(Path('/tmp/current/a.py'), 1, 'changed snippet', 'label')] * 2
        with self.assertRaises(AssertionError):
            m.compare(canonical(original, Path('/tmp/original')), canonical(actual, Path('/tmp/current')))
        with self.assertRaises(AssertionError):
            m.compare(canonical(original), canonical(original[:1]))
        self.assertNotEqual(canonical((1, 2)), canonical([1, 2]))

    def test_source_dto_keeps_every_dataclass_field(self):
        module()
        from dataclasses import dataclass
        from dto import canonical
        @dataclass
        class Finding:
            snippet: str
            extra: str
        self.assertEqual({'snippet': 's', 'extra': 'e'}, canonical(Finding('s', 'e'))['fields'])

    def test_source_dto_normalizes_string_path_prefix_only(self):
        module()
        from dto import canonical
        self.assertEqual('a.py', canonical('/tmp/original/a.py', Path('/tmp/original')))
        self.assertEqual('mentions /tmp/original/a.py', canonical('mentions /tmp/original/a.py', Path('/tmp/original')))

    def test_recorded_results_require_identical_inputs(self):
        m = module()
        recorded = dict(files=[dict(path='a.py', source='first\nsecond\n')], scope=dict(include=['*.py'], exclude=[]))
        with self.assertRaisesRegex(AssertionError, 'input'):
            m.compare_inputs(recorded, [dict(path='a.py', source='second\nfirst\n')], recorded['scope'])

    def test_recorded_results_require_identical_scope(self):
        m = module()
        recorded = dict(files=[], scope=dict(include=['a.py'], exclude=[]))
        with self.assertRaisesRegex(AssertionError, 'scope'):
            m.compare_inputs(recorded, [], dict(include=['b.py'], exclude=[]))
        m.compare_inputs(recorded, [], recorded['scope'])

    def test_full_results_reject_message_changes(self):
        m = module()
        expected = [{'code': 'X', 'message': 'original', 'path': 'a.py', 'line': 1}]
        with self.assertRaises(AssertionError):
            m.compare(expected, [{**expected[0], 'message': 'other'}])

    def test_full_results_reject_lost_duplicates(self):
        m = module()
        expected = [{'code': 'X', 'message': 'same'}] * 2
        with self.assertRaises(AssertionError):
            m.compare(expected, expected[:1])

    def test_full_results_reject_unexpected_codes(self):
        m = module()
        with self.assertRaises(AssertionError):
            m.compare([], [{'code': 'UNEXPECTED'}])

    def test_full_results_reject_missing_dto_fields(self):
        m = module()
        with self.assertRaises(AssertionError):
            m.compare([{'code': 'X', 'symbol': 'f'}], [{'code': 'X'}])

    def test_full_results_keep_list_order(self):
        m = module()
        with self.assertRaises(AssertionError):
            m.compare([{'code': 'A'}, {'code': 'B'}], [{'code': 'B'}, {'code': 'A'}])

    def test_only_explicit_unordered_results_ignore_order(self):
        m = module()
        m.compare([{'code': 'A'}, {'code': 'B'}], [{'code': 'B'}, {'code': 'A'}], unordered=True)

    def test_target_inventory_is_the_existing_hundred(self):
        m = module()
        entries = m.targets(m.ROOT)
        self.assertEqual(100, len(entries))
        self.assertEqual(100, len({e['check'] for e in entries}))
        self.assertEqual(set(range(1, 11)), {e['batch'] for e in entries})
        self.assertTrue(all(e['legacySha256'] for e in entries))

    def test_partial_rule_can_never_be_complete(self):
        m = module()
        for status in ['absent', 'draft', 'reader-passed', 'set-parity-passed']:
            with self.subTest(status=status), self.assertRaises(ValueError):
                m.require_complete([{'check': 'a', 'status': status}])

    def test_completion_needs_assertion_counts_and_bound_sources(self):
        m = module()
        with self.assertRaises(ValueError):
            m.require_complete([{'check': 'a', 'status': 'full-contract-passed'}])

    def test_counts_alone_never_certify_original_assertions(self):
        m = module()
        evidence = dict(assertions=10, positives=3, failures=0, skipped=0,
                        semanticFallbacks=0, ruleSha256='a' * 64,
                        engineCommit='b' * 40, corpusSha256='c' * 64,
                        sourceCommit=m.PIN)
        with self.assertRaises(ValueError):
            m.require_complete([dict(check='a', status='full-contract-passed',
                                     ruleSha256='a' * 64, evidence=evidence)])

    def test_completion_rejects_skips_fallbacks_or_failures(self):
        m = module()
        evidence = dict(assertions=10, positives=3, failures=0, skipped=0,
                        semanticFallbacks=0, ruleSha256='a' * 64,
                        engineCommit='b' * 40, corpusSha256='c' * 64,
                        sourceCommit=m.PIN)
        for key in ['failures', 'skipped', 'semanticFallbacks']:
            with self.subTest(key=key), self.assertRaises(ValueError):
                m.require_complete([dict(check='a', status='full-contract-passed',
                                         evidence={**evidence, key: 1})])


if __name__ == '__main__':
    unittest.main()
