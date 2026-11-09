"""CLI acceptance must retain every original result and reject empty native output."""
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
import raw_execute_cli_contract as contract


class RawExecuteCLIContractTests(unittest.TestCase):
    def recorded(self):
        path=contract.ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-raw-execute-cli-source-v2.json.gz'
        return json.loads(gzip.decompress(path.read_bytes()))

    def run_native(self,root,baseline,execute):
        source=root/'source.json';source.write_text(json.dumps(baseline))
        engine=root/'engine';engine.write_text('frozen engine identity');output=root/'native.json'
        command=['contract','native','--engine',str(engine),'--baseline',str(source),'--output',str(output)]
        with patch.object(sys,'argv',command),patch.object(contract,'execute_workspace',side_effect=execute),contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
            status=contract.main()
        return status,json.loads(output.read_text())

    def test_empty_detector_cannot_pass_positive_cli_cases(self):
        def empty(*args,**kwargs):
            return dict(programInputs=[],diagnostics=[dict(code='RRE_CLI_EXIT',parameters=dict(status=0))])
        with tempfile.TemporaryDirectory() as directory:
            status,report=self.run_native(Path(directory),self.recorded(),empty)
        self.assertEqual(status,1);self.assertTrue(report['complete']);self.assertEqual(len(report['cases']),44)
        self.assertTrue(all(not c['passed'] for c in report['cases'] if c['expected']['violations']))
        self.assertTrue(any(c['expected']['status']==1 for c in report['cases']))

    def test_missing_changed_and_nonpassing_oracles_are_rejected(self):
        for change in ['missing','changed','nonpassing']:
            baseline=self.recorded()
            if change=='missing':baseline['cases'].pop()
            elif change=='changed':baseline['cases'][0]['files'][0]['source']='pass\n'
            else:baseline['passed']=False
            with self.subTest(change=change),tempfile.TemporaryDirectory() as directory:
                with self.assertRaises(SystemExit):self.run_native(Path(directory),baseline,lambda *a,**k:self.fail('Must reject before execution'))

    def test_interruption_cannot_claim_completion(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            with self.assertRaises(TimeoutError):self.run_native(root,self.recorded(),lambda *a,**k:(_ for _ in ()).throw(TimeoutError()))
            report=json.loads((root/'native.json').read_text())
        self.assertFalse(report['complete']);self.assertFalse(report['passed'])
