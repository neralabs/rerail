"""Backend admission retains original positives, empty reads and typed facts."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from backend_types_admission import validate
from migration import MANIFEST,read_json,require_complete


class BackendTypesAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-backend-types-complete-finalec3.json.gz','rt') as stream:cls.value=json.load(stream)

    def test_complete_contract_passes(self):
        result=validate(self.value)
        self.assertEqual(148,result['originalCasesPerAPI']);self.assertEqual(41,result['originalFindingPositiveCases'])
        self.assertEqual(144,result['publicCasesPerAPI']);self.assertEqual(59,result['productionCases'])
        self.assertEqual(865,result['nativeExecutions'])

    def test_lost_backend_positive_is_rejected(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='backend-sqlite-0')
        row['actual']['value']=[]
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_original_assertion_cannot_be_omitted(self):
        value=copy.deepcopy(self.value);value['originalSuites']['snapshot']['cases'].popitem()
        with self.assertRaisesRegex(ValueError,'Original assertions omitted'):validate(value)

    def test_empty_read_is_a_complete_observation(self):
        value=copy.deepcopy(self.value)
        row=next(r for r in value['originalSuites']['snapshot']['detectorObservations'] if 'read_error' in r['test'])
        self.assertEqual('passed',row['status']);self.assertEqual([],row['actual'])
        row['actualSourceResult']=None
        with self.assertRaisesRegex(ValueError,'Original DTO fields'):validate(value)

    def test_bool_fact_cannot_be_replaced_by_integer(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['kind']=='fact' and c['input']['value']['line'] is True)
        row['actual']['value']['line']=1
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_original_observations_need_consumed_native_calls(self):
        value=copy.deepcopy(self.value)
        row=value['originalSuites']['snapshot'];identity=row['detectorObservations'][0]['test']
        row['calls']=[c for c in row['calls'] if c['test']!=identity]
        with self.assertRaisesRegex(ValueError,'Original native execution omitted'):validate(value)

    def test_fixed_inventory_admits_rule(self):
        entry=next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='backend_type_strings')
        require_complete([entry])


if __name__=='__main__':unittest.main()
