"""Source completion order can permute whole file groups, never per-file observations."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
from boundary_port import comparison_value


class BoundaryOrderTests(unittest.TestCase):
    def value(self, rows, count=2):
        return dict(result=dict(kind='dataclass', type='scripts.lint_async_sync_boundary.LintResult',
            fields=dict(violations=[dict(kind='dataclass', type='scripts.lint_async_sync_boundary.Violation',
                fields=dict(file=path, line=line, col=7, category='EXECUTOR_ESCAPE')) for path,line in rows], files_checked=count)))

    def test_only_whole_file_group_order_can_change(self):
        first=self.value([('a.py',2),('a.py',1),('a.py',1),('b.py',3)])
        reordered=self.value([('b.py',3),('a.py',2),('a.py',1),('a.py',1)])
        original=copy.deepcopy(first)
        self.assertEqual(comparison_value(first),comparison_value(reordered))
        self.assertEqual(original,first)
        self.assertNotEqual(comparison_value(first),comparison_value(self.value([('a.py',1),('a.py',2),('a.py',1),('b.py',3)])))
        self.assertNotEqual(comparison_value(first),comparison_value(self.value([('a.py',2),('a.py',1),('b.py',3)])))

    def test_every_other_field_and_error_remains_exact(self):
        first=self.value([('a.py',1)])
        changed=copy.deepcopy(first);changed['result']['fields']['violations'][0]['fields']['col']=8
        self.assertNotEqual(comparison_value(first),comparison_value(changed))
        self.assertNotEqual(comparison_value(first),comparison_value(self.value([('a.py',1)],3)))
        error=dict(error=dict(type='builtins.UnicodeDecodeError',args=['all bytes retained']))
        self.assertEqual(error,comparison_value(error))

    def test_interleaved_file_groups_are_rejected(self):
        with self.assertRaisesRegex(ValueError,'interleaved'):
            comparison_value(self.value([('a.py',1),('b.py',2),('a.py',3)]))
