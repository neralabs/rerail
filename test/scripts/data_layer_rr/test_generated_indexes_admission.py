"""Full admission must retain original positives, legacy inputs, and update evidence."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from generated_indexes_admission import validate


class GeneratedIndexesAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-generated-indexes-complete-finalfcce.json.gz','rt') as stream:
            cls.receipt=json.load(stream)

    def example(self):return copy.deepcopy(self.receipt)

    def test_full_receipt_admits_only_this_rule(self):
        summary=validate(self.receipt)
        self.assertTrue(summary['admitted'])
        self.assertFalse(summary['fullMigrationAcceptance'])
        self.assertEqual(summary['originalPositiveObservationsPerAPI'],4)

    def test_missing_original_positive_is_rejected(self):
        value=self.example();identity=next(r['test'] for r in value['originalSource']['events'] if r['findings'])
        del value['originals']['snapshot']['cases'][identity]
        with self.assertRaisesRegex(ValueError,'Original assertions omitted'):validate(value)

    def test_missing_legacy_duplicate_default_case_is_rejected(self):
        value=self.example()
        for report in value['legacy']['snapshot']:
            report['cases'][:]=[r for r in report['cases'] if r['input']['identity']!='collector-generated_index_sources-duplicate-last-default-clean']
        with self.assertRaisesRegex(ValueError,'Missing or duplicate legacy cases'):validate(value)

    def test_missing_integration_positive_is_rejected(self):
        value=self.example()
        for report in value['integration']['workspace']:
            report['cases'][:]=[r for r in report['cases'] if r['input']['id']!='production-positive']
        with self.assertRaisesRegex(ValueError,'Missing or duplicate integration cases'):validate(value)

    def test_missing_persistent_update_step_is_rejected(self):
        value=self.example();value['updates']['snapshot'][0]['cases'].pop(1)
        with self.assertRaisesRegex(ValueError,'Missing update step'):validate(value)

    def test_weakened_update_expected_value_is_rejected(self):
        value=self.example();row=value['updates']['workspace'][0]['cases'][1]
        row['expected']=row['actual']={}
        with self.assertRaisesRegex(ValueError,'Update result differs'):validate(value)

    def test_native_update_failure_cannot_pass(self):
        value=self.example();value['updates']['snapshot'][0]['complete']=False
        with self.assertRaisesRegex(ValueError,'Incomplete update report'):validate(value)

    def test_invalid_mutant_is_rejected(self):
        value=self.example();value['integrationControls']['cases'][0]['actual']={'error':{'type':'SyntaxError'}}
        with self.assertRaisesRegex(ValueError,'Invalid integration mutant'):validate(value)

    def test_another_update_engine_is_rejected(self):
        value=self.example();value['updates']['workspace'][0]['engineSha256']='other'
        with self.assertRaisesRegex(ValueError,'Incomplete update report'):validate(value)

    def test_missing_native_collector_execution_is_rejected(self):
        value=self.example()
        for report in value['integration']['snapshot']:
            for row in report['cases']:
                if row['input']['id']=='registry-extract-positive':row['native']=[]
        with self.assertRaisesRegex(ValueError,'Missing integration native execution'):validate(value)

    def test_an_old_policy_cannot_hide_among_snapshots(self):
        value=self.example();row=value['legacy']['workspace'][0]['cases'][0]['native'][0]
        for program in row['programInputs']:
            if program['path']==value['rulePath']:program['sha256']='other'
        with self.assertRaisesRegex(ValueError,'Mixed main policy versions'):validate(value)

    def test_missing_generic_regression_is_rejected(self):
        value=self.example();value['retainedWorldProbe']['cases']=[]
        with self.assertRaisesRegex(ValueError,'Failed generic native probe'):validate(value)

    def test_original_recording_cannot_be_changed_after_replay(self):
        value=self.example();value['originalSource']['events'][0]['findings']=[]
        with self.assertRaisesRegex(ValueError,'Changed original oracle bytes'):validate(value)

    def test_original_detector_cannot_fall_back_to_python(self):
        value=self.example();value['originals']['snapshot']['semanticFallbacks']=['original scanner']
        with self.assertRaisesRegex(ValueError,'Original assertions delegated or omitted'):validate(value)


if __name__=='__main__':unittest.main()
