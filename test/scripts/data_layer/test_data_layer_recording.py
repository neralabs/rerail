"""Regression tests for lossless upstream test recording."""
import ast
from pathlib import Path
from types import SimpleNamespace
import sys
import unittest
from weakref import WeakKeyDictionary
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl'))
import data_layer_batch_eight as batch
# Unit tests isolate the pytest registration marker. The unchanged upstream
# suites exercise the real pytest fixture ordering and plugin lifecycle.
def hookimpl(**options):
    def register(method):
        method.pytest_impl = options
        return method
    return register


with patch.dict(sys.modules, {'pytest': SimpleNamespace(hookimpl=hookimpl)}):
    from data_layer_record import Recorder


class Recording(unittest.TestCase):
    def test_setup_fixture_calls_belong_to_the_test(self):
        self.assertTrue(Recorder.pytest_runtest_setup.pytest_impl['tryfirst'])
        recorder = Recorder(Path('/source'), 8)
        item = SimpleNamespace(nodeid='tests.py::positive')
        recorder.pytest_runtest_setup(item)
        self.assertIs(recorder.item, item)
        recorder.pytest_runtest_logreport(SimpleNamespace(nodeid=item.nodeid, when='setup', outcome='passed'))
        self.assertIs(recorder.item, item)
        recorder.pytest_runtest_logreport(SimpleNamespace(nodeid=item.nodeid, when='teardown', outcome='passed'))
        self.assertIsNone(recorder.item)

    def test_ast_input_keeps_original_lines_comments_and_spacing(self):
        source = '\n# original coordinates\n\ndef f( ):\n    return  1\n'
        tree = ast.parse(source)
        sources = WeakKeyDictionary({node: source for node in ast.walk(tree)})
        self.assertEqual(batch.tree_source(tree, sources), source)
        synthetic = ast.Module(body=[tree.body[0]], type_ignores=[])
        self.assertEqual(batch.tree_source(synthetic, sources), source)
        self.assertNotEqual(ast.unparse(tree), source)

    def test_ast_with_ambiguous_or_unknown_source_is_not_reconstructed(self):
        first, second = ast.parse('f()'), ast.parse('g()')
        sources = WeakKeyDictionary({first.body[0]: 'f()', second.body[0]: 'g()'})
        self.assertIsNone(batch.tree_source(ast.Module(body=[first.body[0], second.body[0]], type_ignores=[]), sources))
        self.assertIsNone(batch.tree_source(ast.parse('h()'), sources))

    def test_analyzed_file_domain_uses_path_not_dataclass_identity(self):
        path = Path('/source/l2000_connection/example.py')
        stored = SimpleNamespace(module_name='l2000_connection.example', facts=[])
        self.assertEqual(batch.analyzed_file_sources({path: stored}, {path: 'def f(): pass\n'}), [(path, 'def f(): pass\n')])

    def test_missing_analyzed_source_is_explicit(self):
        with self.assertRaisesRegex(ValueError, 'No original source'):
            batch.analyzed_file_sources({Path('/does-not-exist.py'): object()}, {})


if __name__ == '__main__':
    unittest.main()
