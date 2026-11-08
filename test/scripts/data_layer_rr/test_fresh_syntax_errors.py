"""Syntax failures must retain exception arguments and every source-location field."""
from pathlib import Path
import sys
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from fresh import invoke

class SyntaxErrorTransportTests(unittest.TestCase):
    def test_location_fields_survive_a_source_parse_exception(self):
        def call():raise SyntaxError('invalid syntax',('/fixture/a.py',7,3,'bad\n',7,4))
        value=invoke(call,Path('/fixture'))['error']
        self.assertEqual('builtins.SyntaxError',value['type'])
        self.assertEqual(['invalid syntax',{'kind':'tuple','items':['a.py',7,3,'bad\n',7,4]}],value['args']['items'])
        self.assertEqual(dict(filename='a.py',lineno=7,offset=3,end_lineno=7,end_offset=4,text='bad\n',msg='invalid syntax',print_file_and_line=None),value['attributes'])
