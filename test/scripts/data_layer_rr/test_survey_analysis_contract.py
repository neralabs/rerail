"""Source analysis comparisons retain categories, list order, multiplicity, and explicit refusals."""
from pathlib import Path
import contextlib
import gzip
import io
import json
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from survey_analysis_contract import native_result,same_result
import survey_analysis_contract as contract
import replay


class SurveyAnalysisContractTests(unittest.TestCase):
    def test_complete_result_keeps_category_order_and_symbol_multiplicity(self):
        def row(code,**parameters):return dict(code=code,parameters=parameters)
        rows=[row('IDC_GROUP',category='high',ordinal=1),row('IDC_GROUP',category='unused',ordinal=0),
              row('IDC_ITEM',category='unused',sid='last',ordinal=2),
              row('IDC_ITEM',category='unused',sid='first',ordinal=0),
              row('IDC_ITEM',category='unused',sid='first',ordinal=1)]
        actual=native_result(rows)
        expected=dict(unused=['first','first','last'],high=[])
        self.assertTrue(same_result(expected,actual))
        for changed in [dict(high=[],unused=['first','first','last']),
                        dict(unused=['first','last'],high=[]),
                        dict(unused=['last','first','first'],high=[]),
                        dict(unused=['first','first','last'])]:
            with self.subTest(changed=changed):self.assertFalse(same_result(expected,changed))

    def test_explicit_incomplete_diagnostic_cannot_be_rendered_as_a_clean_result(self):
        with self.assertRaisesRegex(ValueError,'IDC_SOURCE_GRAPH_REQUIRED'):
            native_result([dict(code='IDC_SOURCE_GRAPH_REQUIRED',parameters={})])

    def recording(self):
        path=contract.ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-survey-source-analysis-red.json.gz'
        return json.loads(gzip.decompress(path.read_bytes()))['source']

    def run_native(self,root,baseline,execute):
        source=root/'recording.json';source.write_text(json.dumps(baseline))
        engine=root/'engine';engine.write_text('frozen executable identity')
        output=root/'result.json'
        command=['contract','native','--baseline',str(source),'--engine',str(engine),'--output',str(output)]
        with patch.object(sys,'argv',command),patch.object(replay,'execute_native',side_effect=execute), \
             patch.object(contract,'source_result',side_effect=AssertionError('Source algorithm in native command')), \
             contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
            status=contract.main()
        return status,json.loads(output.read_text())

    def test_native_command_compares_actual_outputs_without_offline_source_execution(self):
        baseline=self.recording()
        headers=[dict(code='IDC_GROUP',parameters=dict(category=name,ordinal=i))
                 for i,name in enumerate(baseline['cases'][0]['expected'])]
        with tempfile.TemporaryDirectory() as temporary:
            status,result=self.run_native(Path(temporary),baseline,lambda *args:dict(diagnostics=headers,programInputs=[]))
        self.assertEqual(status,1)
        self.assertEqual(len(result['cases']),len(contract.CASES))
        self.assertEqual(sum(not c['passed'] for c in result['cases']),102)

    def test_missing_or_modified_source_fixture_cannot_certify_analysis(self):
        for change in ['missing','source']:
            with self.subTest(change=change),tempfile.TemporaryDirectory() as temporary:
                baseline=self.recording()
                if change=='missing':baseline['cases'].pop()
                else:baseline['cases'][0]['files'][0]['source']='different'
                with self.assertRaises(SystemExit):
                    self.run_native(Path(temporary),baseline,lambda *args:self.fail('Execution must not start'))
