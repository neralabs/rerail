"""The native producer contract must compare actual rows and require its full corpus."""
import contextlib
import copy
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
    def test_dictionary_dto_uses_native_order_keys_without_mutating_diagnostics(self):
        rows=[
            dict(parameters=dict(kind='graph_symbol',sid='last',order_file=3,order_local=1)),
            dict(parameters=dict(kind='graph_symbol',sid='first',order_file=0,order_local=9)),
            dict(parameters=dict(kind='graph_symbol',sid='middle',order_file=3,order_local=0)),
            dict(parameters=dict(kind='graph_module',module='later',order_file=4)),
            dict(parameters=dict(kind='graph_module',module='earlier',order_file=1)),
        ]
        before=copy.deepcopy(rows)
        actual=contract.native_rows(rows)
        self.assertEqual(rows,before)
        self.assertEqual({r['sid']:r['ordinal'] for r in actual if r['kind']=='graph_symbol'},
                         dict(first=0,middle=1,last=2))
        self.assertEqual({r['module']:r['ordinal'] for r in actual if r['kind']=='graph_module'},
                         dict(earlier=0,later=1))
        rows[0]['parameters']['order_file']=-1
        self.assertNotEqual(actual,contract.native_rows(rows))

    def test_dictionary_dto_refuses_missing_native_order_evidence(self):
        with self.assertRaises(KeyError):
            contract.native_rows([dict(parameters=dict(kind='graph_symbol',sid='missing',order_file=0))])

    def recording(self):
        value=dict(format='rerail.survey-source-inventory/v1', sourcePin=contract.PIN,
                    cases=[dict(id=name, files=[dict(path=p, source=t) for p,t in files.items()],
                                expected=[dict(kind='module', path=next(iter(files)), module='recorded')])
                           for name,files in contract.CASES])
        by_id={case['id']:case for case in value['cases']}
        for label,owner,name,target,kind,has in contract.TARGET_CONTRACTS:
            by_id[label]['expected'].append(dict(kind='graph_name_call',owner=owner,name=name,target=target,target_kind=kind,has_target=has))
        return value

    def test_source_fixture_cannot_silently_lose_its_positive_branch(self):
        value=self.recording()
        contract.check_target_contracts(value['cases'])
        case=next(c for c in value['cases'] if c['id']=='name-call-method-before-class')
        case['expected'][-1]['target']='a::target'
        case['expected'][-1]['target_kind']='function'
        with self.assertRaisesRegex(ValueError,'method-before-class'):
            contract.check_target_contracts(value['cases'])

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
