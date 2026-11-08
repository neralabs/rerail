"""Tuple arguments must retain their type for hashable frozen source descriptors."""
import sys
import unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from fresh import arguments

class TupleArgumentTests(unittest.TestCase):
    def test_tuple_paths_stay_distinct_from_mutable_list_arguments(self):
        result=arguments(dict(files={'tuple':[{'path':'a.py'},{'path':'b.py'}]},names=['a','b']),Path('/fixture'))
        self.assertEqual((Path('/fixture/a.py'),Path('/fixture/b.py')),result['files'])
        self.assertIsInstance(result['files'],tuple)
        self.assertEqual(['a','b'],result['names'])
        self.assertEqual(hash(result['files']),hash((Path('/fixture/a.py'),Path('/fixture/b.py'))))
