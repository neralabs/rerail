"""Regressions for the PermissionError public API transport."""
from pathlib import Path
import sys
import json
import tempfile
from types import SimpleNamespace
import unittest

ROOT = Path(__file__).absolute().parents[3]
sys.path.insert(0, str(ROOT/'scripts/dsl/data_layer_rr'))


class PermissionPortTests(unittest.TestCase):
    def test_scan_batch_submits_all_candidates_once(self):
        import permission_port
        calls = []
        class Backend:
            def native(self, check, entry, files, options=None):
                calls.append((entry, files, options)); return []
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ['input.py', 'tests/a.py', 'a.txt']:
                p = root/name; p.parent.mkdir(parents=True, exist_ok=True); p.write_text('raise PermissionError\n')
            self.assertEqual(permission_port.scan_batch(Backend(), SimpleNamespace(), root,
                             [Path(p) for p in ['input.py', 'input.py', 'tests/a.py', 'a.txt']]), [])
            self.assertEqual(len(calls), 1)
            self.assertEqual(len(calls[0][1]), 4)
            self.assertEqual(calls[0][2]['candidatePaths'], [str(root/p) for p in ['input.py', 'input.py', 'tests/a.py', 'a.txt']])

    def test_fact_helpers_require_native_conversion(self):
        import permission_port
        calls = []
        class Backend:
            def replace(self, owner, name, value): setattr(owner, name, value)
            def native(self, check, entry, files, options=None):
                calls.append((entry, json.loads(files[0]['source'])))
                return [dict(code='PERMISSION_FACT', parameters=dict(ordinal=0, value_json=json.dumps(
                    dict(file_path='root/a.py', line_number=12, message='message'))))]
        module = SimpleNamespace(_violation_from_fact=None, _violation_to_fact=None,
                                 Violation=lambda *values: values)
        permission_port.install_fact_helpers(Backend(), module)
        raw = dict(path='a.py', line='١٢', message='message')
        self.assertEqual(module._violation_from_fact(raw, Path('root')), (Path('root/a.py'), 12, 'message'))
        self.assertEqual(calls, [('_permission_from_fact', dict(rows=[dict(raw=raw, root='root')]))])

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
