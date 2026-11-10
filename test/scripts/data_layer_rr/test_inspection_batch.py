"""Overlapping scan roots must preserve independent occurrences in one native batch."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from collections import Counter

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class InspectionBatchTests(unittest.TestCase):
    def test_empty_scan_retains_complete_normalized_observation(self):
        from suite import Suite
        import data_layer_record
        suite=Suite.__new__(Suite);identity='test_empty_scan'
        suite.item=SimpleNamespace(nodeid=identity,funcargs={});suite.observed=Counter();suite.observations=[]
        scope=dict(include=['tests/**/*.py'],exclude=['**/__pycache__/**'])
        suite.baseline=dict(events=[dict(test=identity,check='no_source_inspection',entry='find_violations',
            retained_reason='No readable source files (file discovery or I/O contract)',sourceResult=[],findings=[],files=[],scope=scope)])
        with patch.object(data_layer_record,'snapshot',return_value=([],scope,None)):
            suite.compare_original('no_source_inspection','find_violations',SimpleNamespace(),(Path('empty'),),{},[])
        self.assertEqual('passed',suite.observations[0]['status']);self.assertEqual([],suite.observations[0]['actual'])

    def test_overlapping_roots_keep_distinct_document_owners(self):
        from suite import Suite
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):
                calls.append((entry,files,options))
                if entry=='_inspection_prepare':
                    return [dict(code='NSI_READ',parameters=dict(ordinal=i,path=p)) for i,p in enumerate(options['inspectionCandidates']) if Path(p).name=='a.py']
                return []
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'tests').mkdir();(root/'tests/a.py').write_text('inspect.getsource(f)')
            Suite.scan(Backend(),'no_source_inspection','find_violations',SimpleNamespace(TEST_DIRECTORIES=['tests']),
                       dict(root=root,directories=['tests','tests']))
        files=calls[-1][1]
        self.assertEqual(2,len(files));self.assertEqual(2,len({f['path'] for f in files}))
        self.assertEqual(['_inspection_prepare','find_violations'],[r[0] for r in calls])


if __name__=='__main__':unittest.main()
