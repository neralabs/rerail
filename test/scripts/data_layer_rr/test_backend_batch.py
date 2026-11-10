"""The backend CLI must scan its native file batch without per-file queries."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class BackendBatchTests(unittest.TestCase):
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
            module=SimpleNamespace(should_skip_file=lambda p:False)
            self.assertEqual(scan_batch(Backend(),module,paths)[0],[])
        self.assertEqual([c[0] for c in calls],['_bts_prepare','check_file'])
        self.assertEqual(len(calls[1][1]),2)


if __name__=='__main__':unittest.main()
