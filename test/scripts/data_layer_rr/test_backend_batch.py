"""The backend CLI must scan its native file batch without per-file queries."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from collections import Counter

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class BackendBatchTests(unittest.TestCase):
    def test_missing_file_retains_full_empty_observation(self):
        from suite import Suite
        import data_layer_record
        suite=Suite.__new__(Suite)
        identity='test_missing_file'
        suite.item=SimpleNamespace(nodeid=identity,funcargs={})
        suite.observed=Counter();suite.observations=[]
        suite.baseline=dict(events=[dict(test=identity,check='backend_type_strings',entry='check_file',
            retained_reason='No readable source files (file discovery or I/O contract)',sourceResult=[],findings=[],
            files=[],scope=dict(include=['**/*.py'],exclude=[]))])
        suite.calls=[dict(diagnostics=[dict(code='BTS_SKIPPED')])]
        with patch.object(data_layer_record,'snapshot',return_value=([],dict(include=['**/*.py'],exclude=[]),None)):
            suite.compare_original('backend_type_strings','check_file',SimpleNamespace(),(Path('missing.py'),),{},[])
        self.assertEqual('passed',suite.observations[0]['status'])
        self.assertEqual([],suite.observations[0]['actual'])
        self.assertEqual([],suite.observations[0]['expected'])

    def test_scan_batch_has_one_prepare_and_one_scan(self):
        from backend_types_port import scan_batch
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):
                calls.append((entry,files,options))
                if entry=='_bts_prepare':return [dict(code='BTS_READ',parameters=dict(ordinal=i,path=p)) for i,p in enumerate(options['batchPaths'])]
                return []
        with tempfile.TemporaryDirectory() as temporary:
            paths=[Path(temporary)/p for p in ['a.py','b.py']]
            for p in paths:p.write_text('backend_type == "sqlite"')
            module=SimpleNamespace(__name__='source_linter',should_skip_file=lambda p:False)
            self.assertEqual(scan_batch(Backend(),module,paths)[0],[])
        self.assertEqual([c[0] for c in calls],['_bts_prepare','check_file'])
        self.assertEqual(len(calls[1][1]),2)


if __name__=='__main__':unittest.main()
