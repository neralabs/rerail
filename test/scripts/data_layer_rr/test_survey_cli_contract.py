"""CLI acceptance must observe native decisions and reject incomplete recordings."""
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
import survey_cli_contract as contract


class SurveyCliContractTests(unittest.TestCase):
    def baseline(self):
        return json.loads(gzip.decompress((contract.ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-survey-cli-source.json.gz').read_bytes()))

    def run_native(self,root,baseline,execute):
        source=root/'source.json';source.write_text(json.dumps(baseline))
        engine=root/'engine';engine.write_text('frozen engine')
        output=root/'result.json'
        command=['contract','native','--baseline',str(source),'--engine',str(engine),'--output',str(output)]
        with patch.object(sys,'argv',command),patch.object(contract,'native_case',side_effect=execute), \
             patch.object(contract,'source_case',side_effect=AssertionError('Source fallback')), \
             contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
            status=contract.main()
        return status,json.loads(output.read_text())

    def test_correct_analysis_does_not_hide_missing_output_or_status(self):
        for field in ['text','status','dynamicRoots','directories']:
            def execute(engine,rule,item):
                actual=copy.deepcopy(item['expected'])
                actual[field]=None if field=='status' else '' if field=='text' else ['wrong']
                return actual,[]
            with self.subTest(field=field),tempfile.TemporaryDirectory() as temporary:
                status,value=self.run_native(Path(temporary),self.baseline(),execute)
                self.assertEqual(status,1);self.assertTrue(value['complete']);self.assertFalse(value['passed'])

    def test_missing_or_changed_inputs_are_rejected(self):
        for field in ['missing','args','files','complete']:
            value=self.baseline()
            if field=='missing':value['cases'].pop()
            elif field=='complete':value['complete']=False
            elif field=='args':value['cases'][0]['args']=['--check']
            else:value['cases'][0]['files']=[]
            with self.subTest(field=field),tempfile.TemporaryDirectory() as temporary:
                with self.assertRaises(SystemExit):self.run_native(Path(temporary),value,lambda *a:self.fail('Unexpected native execution'))

    def test_interrupted_run_is_not_complete(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            with self.assertRaises(TimeoutError):self.run_native(root,self.baseline(),lambda *a:(_ for _ in ()).throw(TimeoutError()))
            value=json.loads((root/'result.json').read_text())
            self.assertFalse(value['complete']);self.assertFalse(value['passed'])

    def test_input_transport_only_parses_flags_and_paths(self):
        root=Path('/workspace')
        item=next(c for c in contract.CASES if c['id']=='normalized-exclusion')
        options=contract.input_options(item,root)
        self.assertEqual(set(options),{'entryPoint','env','cliJson','cliCheck','cliNoDefaultExcludes','cliExcludePaths','reportLimit'})
        self.assertEqual(options['cliExcludePaths'],[' \u2003impl\\a.py\u2003 '])
        self.assertEqual(options['env'],['data-layer.cli.root=/workspace','data-layer.cli.install=__installation__'])

    def test_context_transport_cannot_supply_selected_documents(self):
        import replay
        with patch.object(replay,'_execute',side_effect=lambda engine,rule,files,options,*a:options(Path('/workspace'))):
            with self.assertRaisesRegex(ValueError,'inputFiles'):
                replay.execute_workspace_context(Path('engine'),Path('rule'),[],lambda root:{'inputFiles':['chosen.py']})
