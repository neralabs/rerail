"""Full boundary admission requires exact positives, frontends, history, edits and controls."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from boundary_admission import validate
import boundary_admission


class ExecutionScope(unittest.TestCase):
    def test_source_metadata_is_not_an_execution_report(self):
        source=dict(results=[dict(programInputs=None)])
        reports=[dict(identity=i) for i in range(6)]
        value=dict(groups=dict(display=dict(source=source,reports=dict(snapshot=[reports[0]],workspace=[reports[1]]))),
            supplemental=dict(report=dict(source=source,reports=[reports[2]])),
            updates=dict(snapshot=[reports[3]],workspace=[reports[4]]),controls=[reports[5]])
        self.assertEqual(list(boundary_admission.execution_reports(value)),reports)
    def test_aggregate_program_inventory_is_not_an_invocation(self):
        call=dict(programInputs=[dict(path='rule.rr',sha256='digest')],phases=[])
        report=dict(programInputs=call['programInputs'],calls=[call])
        self.assertEqual(list(boundary_admission.native_calls(report)),[call])


class Admission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-boundary-complete-finalfcce.json.gz','rt') as f:cls.value=json.load(f)
    def rejected(self,v):
        with self.assertRaises(ValueError):validate(v)
    def test_complete(self):
        s=validate(self.value);self.assertTrue(s['admitted']);self.assertTrue(s['fullRuleMigrationAcceptance']);self.assertFalse(s['fullMigrationAcceptance'])
        self.assertEqual(s['originalCasesPerAPI'],5);self.assertEqual(s['originalPositiveObservationsPerAPI'],3)
        self.assertEqual(s['recordedCasesPerAPI'],173);self.assertEqual(s['scannerCasesPerAPI'],dict(public=32,production=22))
        self.assertEqual(s['frontendCasesPerAPI'],dict(display=31,cli=37,legacy=17));self.assertEqual(s['supplementalCasesSnapshot'],dict(api=295,report=158))
        self.assertEqual(s['persistentStepsPerAPI'],22);self.assertEqual(s['controls'],44)
    def test_missing_original_positive(self):
        v=copy.deepcopy(self.value);v['originals']['snapshot']['detectorObservations'].pop();self.rejected(v)
    def test_missing_registered_positive(self):
        v=copy.deepcopy(self.value)
        for r in v['reports']['workspace']:r['cases'][:]=[c for c in r['cases'] if c['input']['id']!='extract-sync']
        self.rejected(v)
    def test_missing_display(self):
        v=copy.deepcopy(self.value);v['groups']['display']['reports']['snapshot'][0]['cases'].pop();self.rejected(v)
    def test_changed_cli_stdout(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['workspace'][1]['cases'][0]['actual']['stdout']='';self.rejected(v)
    def test_changed_written_baseline(self):
        v=copy.deepcopy(self.value)
        for r in v['groups']['cli']['reports']['snapshot']:
            for c in r['cases']:
                if c['input']['id']=='update':c['actual']['baselineAfter']=None
        self.rejected(v)
    def test_missing_legacy(self):
        v=copy.deepcopy(self.value);v['groups']['legacy']['reports']['snapshot'][0]['cases'].pop();self.rejected(v)
    def test_source_callback(self):
        v=copy.deepcopy(self.value);v['groups']['cli']['reports']['snapshot'][0]['semanticFallbacks']=['cli_main'];self.rejected(v)
    def test_missing_native_rendering(self):
        v=copy.deepcopy(self.value);v['groups']['display']['reports']['workspace'][0]['cases'][0]['native']=[];self.rejected(v)
    def test_missing_old_supplemental(self):
        v=copy.deepcopy(self.value);v['supplemental']['api']['reports'][0]['results'].pop();self.rejected(v)
    def test_missing_update(self):
        v=copy.deepcopy(self.value);v['updates']['snapshot'][0]['cases'].pop();self.rejected(v)
    def test_changed_noop(self):
        v=copy.deepcopy(self.value);v['updates']['workspace'][0]['cases'][2]['changedPaths']=['unrelated.py'];self.rejected(v)
    def test_weakened_update(self):
        v=copy.deepcopy(self.value);r=v['updates']['snapshot'][0]['cases'][1];r['actual']=r['expected']={};self.rejected(v)
    def test_missing_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases']=[];self.rejected(v)
    def test_ineffective_control(self):
        v=copy.deepcopy(self.value);r=v['controls'][0]['cases'][0];r['actual']=r['expected'];self.rejected(v)
    def test_invalid_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases'][0]['actual']={'error':'compile failure'};self.rejected(v)
    def test_invalid_nested_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases'][0]['actual']={'value':{'error':'runtime failure'}};self.rejected(v)
    def test_unrelated_control(self):
        v=copy.deepcopy(self.value);v['controls'][0]['cases'][0]['programCopies']={};self.rejected(v)
    def test_wrong_engine(self):
        v=copy.deepcopy(self.value);v['updates']['workspace'][0]['engineSha256']='other';self.rejected(v)
    def test_wrong_program(self):
        v=copy.deepcopy(self.value);v['programs'][v['rulePath']]['sha256']='other';self.rejected(v)
    def test_null_native_program_inputs(self):
        v=copy.deepcopy(self.value);v['supplemental']['report']['reports'][0]['results'][0]['programInputs']=None;self.rejected(v)
    def test_missing_native_program_inputs(self):
        v=copy.deepcopy(self.value);del v['supplemental']['report']['reports'][0]['results'][0]['programInputs'];self.rejected(v)
    def test_missing_source_order_evidence(self):
        v=copy.deepcopy(self.value);v['sourceCompletionOrder']['runs']=[];self.rejected(v)
    def test_changed_supplemental_count(self):
        v=copy.deepcopy(self.value)
        for report in v['supplemental']['api']['reports']:
            for row in report['results']:
                if row['id']=='overlap':row['value']['result']['fields']['files_checked']=999
        self.rejected(v)


if __name__=='__main__':unittest.main()
