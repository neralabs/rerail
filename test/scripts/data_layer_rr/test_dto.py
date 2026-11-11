"""Exception arguments can contain Python type objects and must stay observable."""
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from dto import canonical


class DTOTests(unittest.TestCase):
    def test_builtin_type_argument_is_retained(self):
        self.assertEqual(canonical(KeyError(tuple).args),dict(kind='tuple',items=[dict(kind='type',name='builtins.tuple')]))

    def test_distinct_builtin_types_stay_distinct(self):
        self.assertNotEqual(canonical(tuple),canonical(list))


if __name__=='__main__':unittest.main()
