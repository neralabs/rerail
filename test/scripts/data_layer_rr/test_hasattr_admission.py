"""Async admission must retain exact positives, all original assertions, and batching."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from hasattr_admission import validate


class HasattrAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-hasattr-complete-finalec3.json.gz','rt') as stream:cls.value=json.load(stream)

    def test_complete_fixed_corpus_and_controls_validate(self):
        result=validate(self.value)
        self.assertEqual((145,17,110,44,16,14,10,705),tuple(result[k] for k in [
            'originalCasesPerAPI','originalFindingPositiveCases','publicCasesPerAPI','productionCases',
            'suppliedFactCases','persistentSteps','controls','nativeExecutions']))

    def test_missing_original_assertion_is_rejected(self):
        value=copy.deepcopy(self.value);value['originalSuites']['snapshot']['cases'].popitem()
        with self.assertRaisesRegex(ValueError,'Original assertions omitted'):validate(value)

    def test_incorrect_native_fact_projection_is_rejected(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='facts-0')
        row['actual']['value']=[]
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_cli_cannot_hide_per_file_queries(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='cli-overlap')
        row['native'].append(copy.deepcopy(row['native'][0]))
        with self.assertRaisesRegex(ValueError,'CLI batch execution count differs'):validate(value)

    def test_missing_positive_control_is_rejected(self):
        value=copy.deepcopy(self.value);value['controls']['cases'].pop()
        with self.assertRaisesRegex(ValueError,'Behavior controls omitted'):validate(value)

    def test_full_recorded_read_error_result_is_required(self):
        value=copy.deepcopy(self.value)
        row=next(r for r in value['originalSuites']['workspace']['detectorObservations'] if 'unicode_decode_error_handled' in r['test'])
        row['actualSourceResult']=None
        with self.assertRaisesRegex(ValueError,'Original DTO fields differ'):validate(value)
