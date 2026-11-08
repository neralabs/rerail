"""Workspace scope acceptance must prove exports without admitting resource symbols."""
import contextlib
import copy
import gzip
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import survey_scope_contract as contract
import replay


class SurveyScopeContractTests(unittest.TestCase):
    def recording(self):
        path=contract.ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-survey-scope-source.json.gz'
        return json.loads(gzip.decompress(path.read_bytes()))

    def test_source_guards_require_export_dead_positive_and_scope_isolation(self):
        original=self.recording()
        contract.check_positives(original['cases'])
        for change in ['export','dead','leak','empty','scanned']:
            with self.subTest(change=change):
                value=copy.deepcopy(original);first=value['cases'][0]
                if change=='export':first['producer']=[r for r in first['producer'] if r['kind']!='graph_export']
                if change=='dead':first['analysis']['unused_functions']=[]
                if change=='leak':first['producer'].append(dict(kind='symbol',sid='data_layer.api::facade_only'))
                if change=='empty':value['cases'][1]['producer']=[dict(kind='symbol',sid='unexpected')]
                if change=='scanned':
                    item=next(c for c in value['cases'] if c['id']=='facade-in-scan')
                    item['producer']=[r for r in item['producer'] if r['kind']!='graph_symbol']
                with self.assertRaises(ValueError):contract.check_positives(value['cases'])

    def run_native(self,root,baseline,execute):
        recording=root/'recording.json';recording.write_text(json.dumps(baseline))
        engine=root/'engine';engine.write_text('frozen executable identity')
        output=root/'result.json'
        command=['contract','native','--baseline',str(recording),'--engine',str(engine),'--output',str(output)]
        with patch.object(sys,'argv',command),patch.object(replay,'execute_workspace',side_effect=execute), \
             patch.object(contract,'source_result',side_effect=AssertionError('Source analysis fallback')), \
             patch.object(contract,'source_rows',side_effect=AssertionError('Source producer fallback')), \
             contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
            status=contract.main()
        return status,json.loads(output.read_text())

    def test_clean_native_detector_fails_positive_cases_without_source_fallback(self):
        baseline=self.recording();categories=list(baseline['cases'][0]['analysis'])
        def native(engine,rule,files,options):
            diagnostics=[] if not options else [dict(code='IDC_GROUP',parameters=dict(category=c,ordinal=i))
                                                 for i,c in enumerate(categories)]
            return dict(diagnostics=diagnostics,programInputs=[])
        with tempfile.TemporaryDirectory() as temporary:
            status,result=self.run_native(Path(temporary),baseline,native)
        self.assertEqual(status,1)
        self.assertTrue(result['complete']);self.assertFalse(result['passed'])
        self.assertEqual(len(result['cases']),len(contract.CASES))
        self.assertFalse(result['cases'][0]['producer']['passed'])
        self.assertFalse(result['cases'][0]['analysis']['passed'])

    def test_incomplete_or_changed_recording_cannot_certify_scope(self):
        for change in ['missing','source','scope','incomplete','failed']:
            with self.subTest(change=change),tempfile.TemporaryDirectory() as temporary:
                value=self.recording()
                if change=='missing':value['cases'].pop()
                if change=='source':value['cases'][0]['files'][0]['source']='different'
                if change=='scope':value['cases'][0]['scanPaths']=[]
                if change=='incomplete':value['complete']=False
                if change=='failed':value['passed']=False
                with self.assertRaises(SystemExit):
                    self.run_native(Path(temporary),value,lambda *args:self.fail('Execution must not start'))

    def test_interrupted_run_cannot_claim_completion(self):
        baseline=self.recording()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            with self.assertRaises(TimeoutError):
                self.run_native(root,baseline,lambda *args:(_ for _ in ()).throw(TimeoutError('interrupted')))
            result=json.loads((root/'result.json').read_text())
        self.assertFalse(result['complete']);self.assertFalse(result['passed'])
        self.assertEqual(result['cases'],[])
