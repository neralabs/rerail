"""The MCP progress checkpoint must retain DTO parity and both unfinished requirements."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from mcp_checkpoint import validate,PENDING


class McpCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-mcp-static-progress-finalec3.json.gz','rt') as stream:cls.value=json.load(stream)

    def test_verified_results_do_not_claim_admission(self):
        result=validate(self.value)
        self.assertEqual(7,result['sharedCasesPerAPI']);self.assertEqual(8,result['requiredSharedCasesPerAPI'])
        self.assertEqual(166,result['compositionPassingPerAPI']);self.assertEqual(370,result['nativeExecutions'])
        self.assertFalse(result['admitted']);self.assertEqual(2,len(result['pending']))

    def test_application_probe_cannot_be_relabelled_as_passed(self):
        value=copy.deepcopy(self.value)
        next(r for r in value['rejections']['snapshot']['cases'][PENDING] if r['phase']=='call')['outcome']='passed'
        with self.assertRaisesRegex(ValueError,'Unrelated shared failure'):validate(value)

    def test_syntax_gap_cannot_be_hidden(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['compositionReports'] for c in r['cases'] if c['input']['id']=='syntax')
        row['passed']=True
        with self.assertRaisesRegex(ValueError,'syntax-location gap'):validate(value)

    def test_full_composition_message_is_required(self):
        value=copy.deepcopy(self.value)
        row=value['reports']['snapshot']['observations'][0]
        row['result']['items'][0]['fields']['message']=''
        with self.assertRaisesRegex(ValueError,'complete result'):validate(value)

    def test_missing_breadth_first_case_is_rejected(self):
        value=copy.deepcopy(self.value)
        for report in value['compositionReports']:
            report['cases']=[c for c in report['cases'] if c['input']['id']!='method-keyword-order']
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(value)

    def test_changed_engine_is_rejected(self):
        value=copy.deepcopy(self.value);value['reports']['workspace']['engineSha256']='other'
        with self.assertRaisesRegex(ValueError,'Mixed native cohort'):validate(value)
