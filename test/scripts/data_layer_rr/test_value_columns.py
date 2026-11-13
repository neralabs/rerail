"""Typed transport preserves value identity, positions and raw float bits."""
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from value_columns import columns


class ValueColumnsTests(unittest.TestCase):
    def test_nested_values_keep_pair_nodes_and_primitive_types(self):
        data=columns({'x':[True,10**80,None,('é',)]})
        self.assertEqual(data['pvKinds'],['dict','pair','str','list','bool','int','NoneType','tuple','str'])
        self.assertEqual(data['pvTexts'][5],str(10**80));self.assertEqual(data['pvTexts'][8],'é')
        self.assertEqual(data['pvFields'][:4],['','','key','value'])
        positions=[int(v)for name in ['pvEnter','pvExit']for v in data[name]]
        self.assertEqual(sorted(positions),list(range(2*len(data['pvKinds']))))
        self.assertEqual(data['pvExit'][0],str(2*len(data['pvKinds'])-1))

    def test_negative_zero_and_nan_payloads_survive_transport(self):
        import struct
        data=columns([-0.0,struct.unpack('>d',bytes.fromhex('7ff8000000000123'))[0]])
        self.assertEqual(data['pvTexts'][1:],['8000000000000000','7ff8000000000123'])

    def test_non_string_dictionary_keys_remain_distinct_typed_values(self):
        data=columns({1:'int','1':'str'})
        self.assertEqual(data['pvKinds'],['dict','pair','int','str','pair','str','str'])
        self.assertEqual(data['pvOrdinals'][1], '0');self.assertEqual(data['pvOrdinals'][4],'1')


if __name__=='__main__':unittest.main()
