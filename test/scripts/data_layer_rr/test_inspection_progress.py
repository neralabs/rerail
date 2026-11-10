"""Verified inspection components must not hide the seven shared integration gaps."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from inspection_progress import validate
from recorded_admission import admit
from migration import MANIFEST,read_json,require_complete


class InspectionProgressTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.checkpoint=ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-inspection-progress-finalec3.json.gz'
        with gzip.open(cls.checkpoint,'rt') as stream:cls.value=json.load(stream)

    def test_verified_components_and_pending_scope_are_explicit(self):
        result=validate(self.value)
        self.assertEqual(125,result['completedOriginalCasesPerAPI']);self.assertEqual(132,result['requiredOriginalCasesPerAPI'])
        self.assertEqual(55,result['originalFindingPositiveCases']);self.assertEqual(7,result['pendingSharedCases'])
        self.assertEqual(1055,result['nativeExecutions']);self.assertFalse(result['admitted'])

    def test_shared_gap_cannot_be_deleted(self):
        value=copy.deepcopy(self.value);value['pendingSharedTests'].pop()
        with self.assertRaisesRegex(ValueError,'Pending shared assertions'):validate(value)

    def test_shared_failure_cannot_be_relabelled_as_success(self):
        value=copy.deepcopy(self.value)
        phases=value['sharedRejection']['cases'][value['pendingSharedTests'][0]]
        next(r for r in phases if r['phase']=='call')['outcome']='passed'
        with self.assertRaisesRegex(ValueError,'Shared failures differ'):validate(value)

    def test_excluded_file_must_not_be_read(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='find-read-spy')
        self.assertFalse(any('__pycache__' in p or '.pytest_cache' in p for p in row['actual']['reads']))
        row['actual']['reads'].append('$TMP/project/tests/__pycache__/bad.py')
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_original_source_observation_cannot_be_omitted(self):
        value=copy.deepcopy(self.value);value['originalSuites']['snapshot']['detectorObservations'].pop()
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(value)

    def test_progress_cannot_be_admitted_or_write_reports(self):
        with patch('recorded_admission.write_json',side_effect=AssertionError('Unexpected write')):
            with self.assertRaisesRegex(ValueError,'Progress receipt cannot be admitted'):admit(self.value,self.checkpoint)

    def test_full_inventory_gate_remains_closed(self):
        entry=next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='no_source_inspection')
        with self.assertRaisesRegex(ValueError,'full assertion evidence is missing'):require_complete([entry])


if __name__=='__main__':unittest.main()
