"""Workspace acceptance must fail on missing positives and altered enforcement."""
import contextlib
import gzip
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import raw_execute_workspace_contract as contract


class RawExecuteWorkspaceContractTests(unittest.TestCase):
    def recorded(self):
        path=contract.ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-raw-execute-source-contracts.json.gz'
        return json.loads(gzip.decompress(path.read_bytes()))

    def run_native(self,root,recorded,execute):
        corpus=root/'cases.json';corpus.write_text(json.dumps(recorded['reporter-cases.json']))
        source=root/'source.json';source.write_text(json.dumps(recorded['reporter-source.json']))
        engine=root/'engine';engine.write_text('frozen engine identity')
        output=root/'native.json'
        command=['contract','--engine',str(engine),'--cases',str(corpus),'--oracle',str(source),'--output',str(output)]
        # Preserve the original corpus bytes hash after temporary serialization.
        value=json.loads(source.read_text());value['casesSha256']=contract.sha(corpus.read_bytes());source.write_text(json.dumps(value))
        with patch.object(sys,'argv',command),patch.object(contract,'execute_workspace',side_effect=execute), \
             contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
            status=contract.main()
        return status,json.loads(output.read_text())

    def native(self,mode='block',rows=()):
        return dict(diagnostics=list(rows),programInputs=[],phases=[],metadata=dict(policies=[dict(id='data-layer-raw-execute-regression',revision=1)],
            settings=[['mode',mode],['description',json.dumps('Removed raw execution surfaces stay removed from production code')],['severity','RRERemoved hard-fail']]))

    def test_empty_detector_cannot_pass_positive_workspace_cases(self):
        with tempfile.TemporaryDirectory() as temporary:
            status,value=self.run_native(Path(temporary),self.recorded(),lambda *a,**k:self.native())
        self.assertEqual(status,1);self.assertTrue(value['complete']);self.assertFalse(value['passed'])
        self.assertEqual(len(value['cases']),31);self.assertEqual(sum(not c['passed'] for c in value['cases']),15)

    def test_correct_findings_cannot_hide_changed_enforcement(self):
        recorded=self.recorded();cases=recorded['reporter-cases.json']['cases'];expected=recorded['reporter-source.json']['results']
        def execute(engine,rule,files,options,**kwargs):
            index=next(i for i,c in enumerate(cases) if c['files']==files)
            rows=[dict(path=r['path'],line=r['line'],code=r['code'],message=r['message'],clause='RRERemoved') for r in expected[index]['value']]
            return self.native('log',rows)
        with tempfile.TemporaryDirectory() as temporary:
            status,value=self.run_native(Path(temporary),recorded,execute)
        self.assertEqual(status,1);self.assertTrue(all(c['actual']==c['expected'] for c in value['cases']))
        self.assertTrue(all(not c['passed'] for c in value['cases']))

    def test_changed_corpus_and_incomplete_metadata_are_rejected_before_execution(self):
        for change in ['missing-case','changed-file','missing-metadata']:
            recorded=self.recorded()
            if change=='missing-case':recorded['reporter-cases.json']['cases'].pop()
            elif change=='changed-file':recorded['reporter-cases.json']['cases'][0]['files'][0]['source']='pass\n'
            else:recorded['reporter-source.json']['results'][0].pop('sourcePlugin')
            with self.subTest(change=change),tempfile.TemporaryDirectory() as temporary:
                with self.assertRaises(SystemExit):self.run_native(Path(temporary),recorded,lambda *a,**k:self.fail('Must reject before execution'))

    def test_interruption_cannot_claim_completion(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            with self.assertRaises(TimeoutError):self.run_native(root,self.recorded(),lambda *a,**k:(_ for _ in ()).throw(TimeoutError()))
            value=json.loads((root/'native.json').read_text())
        self.assertFalse(value['complete']);self.assertFalse(value['passed'])
