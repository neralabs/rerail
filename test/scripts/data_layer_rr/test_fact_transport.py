"""Caller fact transport must retain malformed data and duplicate positions."""
import copy
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from fact_transport import encode_bundles


class FactTransportTests(unittest.TestCase):
    def test_mixed_containers_and_duplicate_rows_survive_transport(self):
        value = [None, [], {}, dict(line=True, message=['☃', {'x': 1}]), dict(line=2**100)]
        original = [dict(path='a.py', value=value + value),
                    dict(path='b.py', value={'wrong-container': value}), dict(path='c.py', value=0)]
        before = copy.deepcopy(original)
        encoded = json.loads(json.dumps(encode_bundles(original)))
        self.assertEqual(original, before)
        self.assertEqual([b['value'] for b in encoded], [b['value'] for b in original])
        self.assertEqual(['array', 'object', 'number'], [b['kind'] for b in encoded])
        self.assertEqual([str(i) for i in range(10)], [r['key'] for r in encoded[0]['rows']])
        self.assertEqual(encoded[0]['rows'][:5], [dict(r, key=str(i)) for i,r in enumerate(encoded[0]['rows'][5:])])
        self.assertEqual(['line', 'message'], [f['key'] for f in encoded[0]['rows'][3]['fields']])
        self.assertIs(encoded[0]['rows'][3]['fields'][0]['value'], True)
        self.assertEqual(2**100, encoded[0]['rows'][4]['fields'][0]['value'])


if __name__ == '__main__': unittest.main()
