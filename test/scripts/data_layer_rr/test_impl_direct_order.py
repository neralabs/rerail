"""Directory enumeration may permute whole files; each file's detector order remains exact."""
import copy
import sys
import unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from impl_direct_execution_contract import comparable

def event(entry,rows):
    return dict(entry=entry,arguments={'root':'same'},return_value=rows)

def row(path,line):
    return dict(kind='dataclass',fields=dict(file=dict(kind='path',value=path),line_num=line,pattern='p',line='source'))

class DirectoryOrder(unittest.TestCase):
    def call(self,entry,rows):
        value=event(entry,rows);value['return']=value.pop('return_value');return value
    def test_only_whole_groups_may_permute(self):
        a=self.call('scan_directory',[row('b',2),row('b',3),row('a',1)])
        b=self.call('scan_directory',[row('a',1),row('b',2),row('b',3)])
        original=copy.deepcopy(a)
        self.assertEqual(comparable(a),comparable(b));self.assertEqual(a,original)
    def test_order_inside_file_and_multiplicity_stay_exact(self):
        a=self.call('scan_directories',[row('b',2),row('b',3),row('a',1),row('a',1)])
        for rows in [[row('b',3),row('b',2),row('a',1),row('a',1)],[row('b',2),row('b',3),row('a',1)]]:
            self.assertNotEqual(comparable(a),comparable(self.call('scan_directories',rows)))
    def test_single_file_and_argument_order_do_not_change(self):
        a=self.call('scan_file',[row('a',2),row('a',3)])
        b=self.call('scan_file',[row('a',3),row('a',2)])
        self.assertNotEqual(comparable(a),comparable(b))
        b=copy.deepcopy(a);b['arguments']['root']='other'
        self.assertNotEqual(comparable(a),comparable(b))
