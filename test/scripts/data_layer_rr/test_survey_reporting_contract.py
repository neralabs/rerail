"""Reporting parity requires native formatting and never supplies expected analysis rows."""
import contextlib
import copy
import gzip
import io
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import survey_reporting_source_contract as contract
from internal_dead_code_port import graph_columns


class SurveyReportingContractTests(unittest.TestCase):
    def baseline(self):
        path=contract.ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-survey-source-report-source.json.gz'
        return json.loads(gzip.decompress(path.read_bytes()))

    def run_native(self,root,baseline,execute):
        source=root/'source.json';source.write_text(json.dumps(baseline))
        engine=root/'engine';engine.write_text('frozen engine identity')
        output=root/'native.json'
        command=['contract','native','--baseline',str(source),'--engine',str(engine),'--output',str(output)]
        with patch.object(sys,'argv',command),patch.object(contract,'native_case',side_effect=execute), \
             patch.object(contract,'source_case',side_effect=AssertionError('Source reporting fallback')), \
             contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
            status=contract.main()
        return status,json.loads(output.read_text())

    def test_matching_analysis_with_empty_native_report_fails_every_case(self):
        def execute(engine,rule,item):
            result=copy.deepcopy(item['expected']);result['text']='';return result,[]
        with tempfile.TemporaryDirectory() as temporary:
            status,result=self.run_native(Path(temporary),self.baseline(),execute)
        self.assertEqual(status,1);self.assertFalse(result['passed']);self.assertTrue(result['complete'])
        self.assertEqual(len(result['cases']),16)
        self.assertTrue(all(not c['passed'] for c in result['cases']))

    def test_missing_cases_or_changed_report_inputs_are_rejected_before_execution(self):
        for change in ['missing','mode','limit','exclusions','incomplete']:
            with self.subTest(change=change),tempfile.TemporaryDirectory() as temporary:
                value=self.baseline()
                if change=='missing':value['cases'].pop()
                if change=='mode':value['cases'][0]['mode']='json'
                if change=='limit':value['cases'][0]['limit']=123
                if change=='exclusions':value['cases'][0]['excludedPaths']=['impl']
                if change=='incomplete':value['complete']=False
                with self.assertRaises(SystemExit):self.run_native(Path(temporary),value,lambda *args:self.fail('Must reject before execution'))

    def test_interruption_cannot_claim_complete_report_acceptance(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            with self.assertRaises(TimeoutError):self.run_native(root,self.baseline(),lambda *args:(_ for _ in ()).throw(TimeoutError('interrupted')))
            result=json.loads((root/'native.json').read_text())
        self.assertFalse(result['complete']);self.assertFalse(result['passed'])

    def test_native_source_transport_has_no_expected_results_or_graph_arguments(self):
        import replay
        item=contract.CASES[0]
        with patch.object(replay,'execute_workspace',return_value=dict(diagnostics=[],programInputs=[])) as execute:
            contract.native_case(Path('engine'),Path('rule'),item)
        options=execute.call_args.args[3]
        self.assertEqual(set(options),{'entryPoint','reportLimit','excludedPaths','env'})
        self.assertEqual(options['entryPoint'],'report_text')

    def test_graph_transport_retains_actual_qualname_instead_of_deriving_it_from_sid(self):
        graph=SimpleNamespace(symbols={'m::f':SimpleNamespace(qualname='Different.actual.qualifier')},
                              symbol_by_fqname={},class_methods={},modules={},exported_root_ids=set())
        values,documents=graph_columns(graph)
        self.assertEqual(values['symbolQualnames'],['Different.actual.qualifier'])
        self.assertEqual(documents,[])
