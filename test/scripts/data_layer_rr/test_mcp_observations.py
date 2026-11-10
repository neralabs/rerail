"""Shared tests must compare complete DTOs, argument values and call multiplicity."""
import copy
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from mcp_observations import validate


class McpObservationsTests(unittest.TestCase):
    def setUp(self):
        self.expected=[dict(test='shared-test',check='composition',entry='scan',ordinal=0,
            args=dict(kind='tuple',items=['source']),kwargs=dict(kind='dict',items=[]),
            result=dict(kind='tuple',items=[dict(rule='SPC-001',line=1,message='complete message')]))]

    def test_complete_observation_passes(self):validate(self.expected,copy.deepcopy(self.expected))

    def test_lost_message_is_rejected(self):
        actual=copy.deepcopy(self.expected);actual[0]['result']['items'][0]['message']=''
        with self.assertRaisesRegex(ValueError,'complete result'):validate(self.expected,actual)

    def test_changed_source_argument_is_rejected(self):
        actual=copy.deepcopy(self.expected);actual[0]['args']['items'][0]='other'
        with self.assertRaisesRegex(ValueError,'arguments'):validate(self.expected,actual)

    def test_tuple_cannot_be_replaced_with_list(self):
        actual=copy.deepcopy(self.expected);actual[0]['result']=actual[0]['result']['items']
        with self.assertRaisesRegex(ValueError,'complete result'):validate(self.expected,actual)

    def test_repeated_observation_is_rejected(self):
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(self.expected,self.expected*2)

    def test_missing_observation_is_rejected(self):
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(self.expected,[])
