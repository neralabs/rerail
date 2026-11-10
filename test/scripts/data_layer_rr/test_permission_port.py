"""Regressions for the PermissionError public API transport."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

ROOT = Path(__file__).absolute().parents[3]
sys.path.insert(0, str(ROOT/'scripts/dsl/data_layer_rr'))


class PermissionPortTests(unittest.TestCase):
    def test_content_uses_disk_and_passes_trigger_to_native(self):
        import permission_port
        calls = []
        class Backend:
            def replace(self, owner, name, value): setattr(owner, name, value)
            def native(self, check, entry, files, options=None):
                calls.append((entry, files, options))
                return []
        module = SimpleNamespace(check_file_content=None, check_file=None)
        permission_port.install_detectors(Backend(), module)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'input.py'; path.write_text('pass\n')
            self.assertEqual(module.check_file_content(path, 'raise PermissionError\n'), [])
        self.assertEqual(calls, [('check_file_content', [dict(path='input.py', source='pass\n')],
                                 dict(permissionSuppliedSource='raise PermissionError\n'))])


if __name__ == '__main__': unittest.main()
