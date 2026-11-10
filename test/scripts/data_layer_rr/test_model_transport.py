"""Model transport retains raw values, unknown fields and nested member order."""
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from model_transport import encode_models


class ModelTransportTests(unittest.TestCase):
    def test_complete_tree_is_retained_without_model_decisions(self):
        raw={'unknown':[None,{'a':True,'b':2**100}],'path':[]}
        result=json.loads(json.dumps(encode_models([dict(path='test.py',value=raw)])))[0]
        self.assertEqual(raw,result['value'])
        nodes=result['nodes'];self.assertEqual(raw,nodes[0]['raw'])
        self.assertEqual(list(range(len(nodes))),[r['id'] for r in nodes])
        self.assertEqual([('unknown',0),('path',1)],[(r['key'],r['ordinal']) for r in nodes if r['parent']==0])
        branch=next(r['id'] for r in nodes if r['raw']=={'a':True,'b':2**100})
        self.assertEqual([('a',True),('b',2**100)],[(r['key'],r['raw']) for r in nodes if r['parent']==branch])
        self.assertTrue(any(r['raw'] is None for r in nodes))
        self.assertTrue(any(r['raw']==[] for r in nodes))


if __name__=='__main__':unittest.main()
