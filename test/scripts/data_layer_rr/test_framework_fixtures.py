"""Framework metadata setup must not relax the original-detector guard."""
from dataclasses import dataclass
from pathlib import Path
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).absolute().parents[3] / 'scripts/dsl/data_layer_rr'))
from framework_fixtures import prepare, GOVERNANCE_NAMES
from suite import Suite


@dataclass
class Registration:
    names: tuple
    factory: object


class FrameworkFixtureTests(unittest.TestCase):
    def test_other_suites_do_not_import_framework_fixtures(self):
        with patch('framework_fixtures.importlib.import_module') as load:
            prepare(None, ['scripts/tests/test_lint_dynamic_module_loading.py'])
        load.assert_not_called()

    def test_metadata_setup_preserves_detector_rejection(self):
        with tempfile.TemporaryDirectory() as temporary:
            backend = Suite.__new__(Suite)
            backend.source = Path(temporary)
            backend.checks = []
            backend.originals = []
            backend.semantic_fallbacks = []
            backend.host_protocol_calls = []
            backend.item = None
            descriptor = object()
            registrations = [Registration((name,), lambda: descriptor) for name in sorted(GOVERNANCE_NAMES)]
            framework = SimpleNamespace(DL_SUITE=SimpleNamespace(linter_plugin_factories=registrations))
            standalone = SimpleNamespace(STANDALONE_LINTER_COMMANDS=(('lint_example.py', ()),),
                                         registered_standalone_linter_scripts=lambda: None)
            modules = {'scripts.lint_all': framework, 'scripts.lint_standalone_linter_entrypoints_plugin': standalone}
            namespace = {}
            exec(compile('def scan():\n return []\n', str(backend.source / 'scripts/lint_neighbor.py'), 'exec'), namespace)
            with patch('framework_fixtures.importlib.import_module', side_effect=modules.__getitem__):
                prepare(backend, ['scripts/tests/test_lint_all.py'])
            self.assertEqual(standalone.registered_standalone_linter_scripts(), {'lint_example.py'})
            self.assertIsNot(framework.DL_SUITE.linter_plugin_factories, registrations)
            self.assertTrue(all(r.factory() is descriptor for r in framework.DL_SUITE.linter_plugin_factories))
            try:
                backend.pytest_configure()
                with self.assertRaisesRegex(AssertionError, 'Unselected original linter'):
                    namespace['scan']()
            finally:
                sys.setprofile(None)
                threading.setprofile(None)
            self.assertEqual(backend.semantic_fallbacks, ['neighbor:scan'])
