"""The external file adapter must preserve native exclusions before file I/O."""
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
from async_patterns_port import scan


class AsyncTransportTests(unittest.TestCase):
    def test_native_exclusion_precedes_file_access(self):
        suite = SimpleNamespace(native=Mock(return_value=[dict(code='AP_EXCLUDED')]))
        with patch.object(Path, 'read_text', side_effect=AssertionError('Excluded file was opened')):
            self.assertEqual([], scan(suite, 'lint_file', None, dict(filepath=Path('api/test_input.py'))))
        suite.native.assert_called_once_with('async_patterns', 'should_exclude', [], dict(originalPath='api/test_input.py'))

    def test_nonexcluded_file_keeps_original_io_exception(self):
        error = FileNotFoundError(2, 'Missing fixture', 'api/missing.py')
        suite = SimpleNamespace(native=Mock(side_effect=[[], [dict(code='AP_READ_ERROR')]]))
        with patch.object(Path, 'read_text', side_effect=error):
            with self.assertRaises(FileNotFoundError) as caught:
                scan(suite, 'lint_file', None, dict(filepath=Path('api/missing.py')))
        self.assertIs(error, caught.exception)
        self.assertEqual(['should_exclude', 'lint_file'], [call.args[1] for call in suite.native.call_args_list])
