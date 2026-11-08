"""Only permutations of complete source file groups may ignore worker completion order."""
import sys, unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import patterns_contract


class PatternOrderTests(unittest.TestCase):
    def row(self,path,line):
        return dict(kind='dict',items=[['file',path],['line',line],['name','complete finding']])

    def test_complete_file_groups_can_change_completion_order(self):
        a,b,c=self.row('a.py',2),self.row('a.py',1),self.row('b.py',3)
        self.assertEqual(patterns_contract.file_groups([a,b,c]),patterns_contract.file_groups([c,a,b]))

    def test_fields_duplicates_and_in_file_order_remain_exact(self):
        a,b=self.row('a.py',2),self.row('a.py',1)
        expected=patterns_contract.file_groups([a,b,b])
        for changed in [[a,b],[b,a,b],[a,b,self.row('a.py',9)]]:
            self.assertNotEqual(expected,patterns_contract.file_groups(changed))

    def test_interleaved_files_are_rejected(self):
        with self.assertRaisesRegex(ValueError,'contiguous'):
            patterns_contract.file_groups([self.row('a.py',1),self.row('b.py',2),self.row('a.py',3)])
