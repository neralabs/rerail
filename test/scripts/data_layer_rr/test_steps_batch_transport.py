"""Batched step observations must retain positive DTOs and configuration boundaries."""
import ast
from pathlib import Path
import sys
import types
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import serverplan_steps_port as port
from serverplan_steps_contract import FUNCTIONS


class StepsBatchTransportTests(unittest.TestCase):
    def setUp(self):
        class Violation:
            def __init__(self, code, file, line, message):
                self.code, self.file, self.line, self.message = code, file, line, message
            def __str__(self):
                raise AssertionError('Use the native formatter')
        module = types.SimpleNamespace(Violation=Violation, ROOT=Path('/source'),
            CAN_BATCH_FALSE_ALLOWLIST=frozenset(), LIFECYCLE_RUNTIME_STATE_ALLOWLIST=frozenset(),
            LEGACY_SIDE_EFFECT_TAG_ALLOWLIST=frozenset(), ALLOWLIST_SIGNATURE='signature')
        for name in FUNCTIONS:
            setattr(module, name, lambda *a, **k: self.fail('Source detector invoked'))
        class Backend:
            def __init__(self):
                self.originals, self.calls = [], []
            def replace(self, owner, name, value):
                self.originals.append((owner, name, getattr(owner, name)))
                setattr(owner, name, value)
            def native(self, check, entry, files, options=None):
                self.calls.append((check, entry, files, options))
                def row(entry, code, message):
                    return dict(code='SSP_ROW', path='a.py', line=3, parameters=dict(
                        entry=entry, violation_code=code, detail=message, class_order=1,
                        method_order=2, callback_order=0, node_order=3, suborder=0))
                return [row('check_compile_no_object_manipulation', 'STEP-017', 'object positive'),
                        row('check_compile_no_data_branching', 'STEP-016', 'branch positive')]
        self.backend, self.module = Backend(), module
        self.addCleanup(self.restore)
        port.install(self.backend, module)

    def restore(self):
        for owner, name, value in reversed(self.backend.originals):
            setattr(owner, name, value)

    def test_one_batch_preserves_each_positive_and_the_empty_check(self):
        tree = ast.parse('value = 1\n')
        first = self.module.check_compile_no_data_branching(Path('a.py'), tree)
        second = self.module.check_compile_no_object_manipulation(Path('a.py'), tree)
        empty = self.module.check_can_batch_allowlist(Path('a.py'), tree)
        self.assertEqual([dict(code='STEP-016',file=Path('a.py'),line=3,message='branch positive')],
                         [vars(v) for v in first])
        self.assertEqual(['STEP-017'], [v.code for v in second])
        self.assertEqual([], empty)
        self.assertEqual(1, len(self.backend.calls))
        self.assertEqual('_all_checks', self.backend.calls[0][1])
        self.assertEqual([dict(path='input.py',source='value = 1\n')], self.backend.calls[0][2])

    def test_changed_configuration_source_and_path_cannot_reuse_a_batch(self):
        tree = ast.parse('value = 1\n')
        check = self.module.check_compile_no_data_branching
        check(Path('a.py'), tree)
        self.module.CAN_BATCH_FALSE_ALLOWLIST = frozenset({'ExceptionStep'})
        check(Path('a.py'), tree)
        check(Path('b.py'), tree)
        check(Path('b.py'), ast.parse('value = 2\n'))
        self.assertEqual(4, len(self.backend.calls))
        self.assertEqual(['ExceptionStep'], self.backend.calls[1][3]['canBatchAllowlist'])
        self.assertEqual('b.py', self.backend.calls[2][3]['argumentPath'])

    def test_changed_tree_cannot_use_the_original_source_or_a_cached_batch(self):
        tree = ast.parse('value = 1\n')
        self.module.check_compile_no_data_branching(Path('a.py'), tree)
        tree.body[0].value = ast.Constant(value=2)
        with self.assertRaisesRegex(NotImplementedError, 'Changed AST'):
            self.module.check_compile_no_data_branching(Path('a.py'), tree)
        self.assertEqual(1, len(self.backend.calls))


if __name__ == '__main__':
    unittest.main()
