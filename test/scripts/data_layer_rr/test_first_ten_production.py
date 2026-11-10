"""Production comparisons must preserve each file's facts and ordering."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from first_ten_production_contract import canonical_findings, normalize


class ProductionComparisonTests(unittest.TestCase):
    def test_unspecified_file_groups_compare_path_and_string_keys_equally(self):
        paths = ['impl/a/z.py', 'impl/a-.py', 'impl/a.py']
        source = [dict(path=Path('/fixture')/p, message=p) for p in paths]
        native = [dict(path='/fixture/'+p, message=p) for p in reversed(paths)]
        self.assertEqual(normalize(canonical_findings('removed_compat_surfaces', source), Path('/fixture')),
                         normalize(canonical_findings('removed_compat_surfaces', native), Path('/fixture')))

    def test_group_normalization_keeps_duplicates_and_in_file_order(self):
        a = dict(path='a.py', message='second'); b = dict(path='a.py', message='first')
        other = dict(path='b.py', message='other')
        result = canonical_findings('removed_compat_surfaces', [other, a, b, b])
        self.assertEqual(result, [a, b, b, other])
        self.assertNotEqual(result, canonical_findings('removed_compat_surfaces', [other, b, a, b]))

    def test_ordered_reporters_do_not_gain_file_group_normalization(self):
        rows = [dict(path='z.py'), dict(path='a.py')]
        self.assertEqual(canonical_findings('hasattr_async', rows), rows)


if __name__ == '__main__': unittest.main()
