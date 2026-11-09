"""A ratchet checkpoint must retain positive cases, exact streams, and input hashes."""
import copy
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import validate_impl_direct_ratchet as checkpoint


class RatchetCheckpoint(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.engine=Path(self.temporary.name)/'engine'
        self.engine.write_bytes(b'fixture')
        self.cases=[dict(id='clean',count=0),dict(id='positive',count=1)]
        self.source=dict(mode='source',passed=True,sourcePin=checkpoint.PIN,sourceFiles={},
            cases=[dict(input=c,expected=dict(stdout='finding\n' if c['count'] else '',status=c['count'])) for c in self.cases])
        program=Path(checkpoint.__file__)
        self.reports=[]
        for mode in ['snapshot','workspace']:
            self.reports.append(dict(mode='native',passed=True,sourcePin=checkpoint.PIN,
                engineSha256=checkpoint.sha(b'fixture'),sourceFiles={},harnessFiles={},executionMode=mode,
                selection=dict(part=0,parts=1),cases=[dict(c,actual=c['expected'],passed=True,
                    native=[dict(programInputs=[dict(path=str(program),sha256=checkpoint.sha(program.read_bytes()))])],signing=[])
                    for c in copy.deepcopy(self.source['cases'])]))
        for name,value in [('CASES',self.cases),('read_json',lambda _:dict(engineSource={})),
                ('verify_source_cohort',lambda *_:None),('verify_engine_artifact',lambda *_:None)]:
            patcher=patch.object(checkpoint,name,value)
            self.addCleanup(patcher.stop)
            patcher.start()

    def validate(self):return checkpoint.validate(self.source,self.reports,self.engine)

    def test_complete_two_api_contract(self):
        self.assertEqual(self.validate()['summary']['verifiedNativeExecutions'],4)

    def test_positive_case_cannot_be_dropped(self):
        self.reports[0]['cases'].pop()
        with self.assertRaises((AssertionError,ValueError)):self.validate()

    def test_duplicate_reports_do_not_cover_a_missing_api(self):
        self.reports[1]=copy.deepcopy(self.reports[0])
        with self.assertRaises((AssertionError,ValueError)):self.validate()

    def test_positive_output_cannot_be_normalized_away(self):
        self.reports[0]['cases'][1]['actual']=dict(stdout='',status=1)
        with self.assertRaises((AssertionError,ValueError)):self.validate()

    def test_changed_policy_bytes_invalidate_the_checkpoint(self):
        self.reports[0]['cases'][0]['native'][0]['programInputs'][0]['sha256']='stale'
        with self.assertRaises((AssertionError,ValueError)):self.validate()
