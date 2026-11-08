"""The native producer contract must compare actual rows and require its full corpus."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import survey_source_contract as contract
import replay


class SurveySourceContractTests(unittest.TestCase):
    def recording(self):
        return dict(format='rerail.survey-source-inventory/v1', sourcePin=contract.PIN,
                    cases=[dict(id=name, files=[dict(path=p, source=t) for p,t in files.items()],
                                expected=[dict(kind='module', path=next(iter(files)), module='recorded')])
                           for name,files in contract.CASES])

    def run_native(self, root, value, actual):
        baseline=root/'source.json'; baseline.write_text(json.dumps(value))
        engine=root/'engine'; engine.write_text('engine identity')
        output=root/'result.json'
        command=['contract','native','--baseline',str(baseline),'--engine',str(engine),'--output',str(output)]
        with patch.object(sys,'argv',command), patch.object(replay,'execute_native',side_effect=actual), \
             patch.object(contract,'source_rows',side_effect=AssertionError('Python fallback')), \
             contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            status=contract.main()
        return status,json.loads(output.read_text())

    def test_native_compares_actual_rows_without_source_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            value=self.recording()
            def native(engine,rule,files,options):
                return dict(diagnostics=[dict(parameters=dict(kind='module',path=files[0]['path'],module='different'))],programInputs=[])
            status,result=self.run_native(Path(directory),value,native)
        self.assertEqual(status,1)
        self.assertTrue(all(not c['passed'] for c in result['cases']))
        self.assertEqual(len(result['cases']),len(contract.CASES))

    def test_missing_case_or_changed_source_cannot_certify_corpus(self):
        for change in ['missing','source','empty']:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                value=self.recording()
                if change=='missing':value['cases'].pop()
                elif change=='source':value['cases'][0]['files'][0]['source']='different'
                else:value['cases'][0]['expected']=[]
                with self.assertRaises(SystemExit):
                    self.run_native(Path(directory),value,lambda *args:self.fail('Native must not start'))
