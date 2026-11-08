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
        for label,owner,ref_kind,base,attr,target,kind,has in contract.MEMBER_CONTRACTS:
            by_id[label]['expected'].append(dict(kind='graph_ref',owner=owner,ref_kind=ref_kind,base=base,attr=attr,target=target,target_kind=kind,has_target=has))
        for label,kind,owner,sid in contract.EDGE_CONTRACTS:
            by_id[label]['expected'].append(dict(kind=kind,owner=owner,sid=sid))
        return value

    def test_source_fixture_cannot_silently_lose_its_positive_branch(self):
        value=self.recording()
        contract.check_target_contracts(value['cases'])
        case=next(c for c in value['cases'] if c['id']=='name-call-method-before-class')
        case['expected'][-1]['target']='a::target'
        case['expected'][-1]['target_kind']='function'
        with self.assertRaisesRegex(ValueError,'method-before-class'):
            contract.check_target_contracts(value['cases'])

    def test_member_and_edge_positive_guards_refuse_missing_observations(self):
        for label in {c[0] for c in contract.MEMBER_CONTRACTS+contract.EDGE_CONTRACTS}:
            with self.subTest(case=label):
                value=self.recording()
                case=next(c for c in value['cases'] if c['id']==label)
                case['expected']=[r for r in case['expected'] if r['kind']=='module']
                with self.assertRaisesRegex(ValueError,label):
                    contract.check_target_contracts(value['cases'])

    def run_native(self, root, value, actual, extra=()):
        baseline=root/'source.json'; baseline.write_text(json.dumps(value))
        engine=root/'engine'; engine.write_text('engine identity')
        output=root/'result.json'
        command=['contract','native','--baseline',str(baseline),'--engine',str(engine),'--output',str(output),*extra]
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

    def test_shards_partition_full_validated_corpus_without_dropping_or_repeating_cases(self):
        seen=[]
        for part in range(3):
            with tempfile.TemporaryDirectory() as directory:
                value=self.recording()
                def native(engine,rule,files,options):
                    return dict(diagnostics=[],programInputs=[])
                status,result=self.run_native(Path(directory),value,native,['--part',str(part),'--parts','3'])
                self.assertEqual(status,1)
                self.assertEqual(result['caseSelection'],dict(part=part,parts=3,total=len(contract.CASES)))
                self.assertEqual([c['id'] for c in result['cases']], [c['id'] for c in value['cases'][part::3]])
                seen.extend(c['id'] for c in result['cases'])
        self.assertCountEqual(seen,[name for name,_ in contract.CASES])
