"""The async capability CLI must filter and scan each whole demand batch."""
from collections import Counter
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class HasattrBatchTests(unittest.TestCase):
    def test_one_selection_and_scan_preserve_duplicate_owners(self):
        from hasattr_port import check_files
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):
                calls.append((entry,files,options))
                if entry=='_async_prepare':return [dict(code='ASYNC_READ',parameters=dict(ordinal=i,path=options['asyncPaths'][i])) for i in [0,2]]
                return [dict(code='hasattr',path=row['path'],line=2,parameters=dict(method='insert_async')) for row in files]
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);a=root/'a.py';a.write_text('hasattr(txn,"insert_async")')
            result=check_files(Backend(),SimpleNamespace(),[a,root/'tests/never-read.py',a])
            self.assertEqual([(a,2,'hasattr','insert_async')]*2,result)
        self.assertEqual(['_async_prepare','check_file'],[c[0] for c in calls])
        self.assertEqual(2,len(calls[1][1]));self.assertEqual(2,len({r['path'] for r in calls[1][1]}))

    def test_missing_file_still_gets_a_native_empty_execution(self):
        from hasattr_port import scan
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):calls.append((entry,files));return []
        self.assertEqual([],scan(Backend(),'check_file',dict(filepath=Path('/unreadable-missing.py'))))
        self.assertEqual([('check_file',[])],calls)

    def test_empty_original_file_result_is_a_complete_observation(self):
        from suite import Suite
        import data_layer_record
        suite=Suite.__new__(Suite);identity='test_missing_file'
        suite.item=SimpleNamespace(nodeid=identity,funcargs={});suite.observed=Counter();suite.observations=[]
        suite.baseline=dict(events=[dict(test=identity,check='hasattr_async',entry='check_file',
            retained_reason='No readable source files (file discovery or I/O contract)',sourceResult=[],findings=[],
            files=[],scope=dict(include=['**/*.py'],exclude=[]))])
        with patch.object(data_layer_record,'snapshot',return_value=([],dict(include=['**/*.py'],exclude=[]),None)):
            suite.compare_original('hasattr_async','check_file',SimpleNamespace(),(Path('missing.py'),),{},[])
        self.assertEqual('passed',suite.observations[0]['status'])
