"""Lifecycle admission requires all original calls and the complete portable contract."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from lifecycle_admission import validate
from migration import MANIFEST,read_json,require_complete


class LifecycleAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-lifecycle-complete-finalec3.json.gz','rt') as stream:
            cls.value=json.load(stream)

    def test_complete_contract_passes(self):
        result=validate(self.value)
        self.assertEqual(142,result['publicCasesPerAPI']);self.assertEqual(1,result['originalCasesPerAPI'])
        self.assertEqual(19,result['productionCases']);self.assertEqual(11,result['controls'])

    def test_omitted_original_negative_call_is_rejected(self):
        value=copy.deepcopy(self.value);value['originalSuites']['snapshot']['detectorObservations'].pop()
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(value)

    def test_nested_collector_order_cannot_be_replaced_by_line_order(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='nested-order')
        row['actual']['value'].sort(key=lambda v:v['line'])
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_lost_positive_is_rejected(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='BackendVersionedWriteTransaction:commit:')
        row['actual']['value']=[]
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_fact_formatting_control_must_change_the_expected_part(self):
        value=copy.deepcopy(self.value)
        row=next(r for r in value['controls']['cases'] if r['name']=='fact-split')
        value_row=next(r for r in row['disabled']['diagnostics'] if r['code']=='LC_VALUE')
        value_row['parameters']['fact_json']='{"message":"unrelated"}'
        with self.assertRaisesRegex(ValueError,'Fact splitting control'):validate(value)

    def test_fixed_original_inventory_admits_rule(self):
        entry=next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='versioned_lifecycle_legacy_surfaces')
        require_complete([entry])


if __name__=='__main__':unittest.main()
