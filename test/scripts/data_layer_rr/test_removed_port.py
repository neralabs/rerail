"""The RemovedCompat adapter must submit scans as whole native batches."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

ROOT=Path(__file__).absolute().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class RemovedPortTests(unittest.TestCase):
    def test_scan_submits_one_batch_and_uses_native_paths(self):
        import removed_port
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):
                calls.append((entry,files,options))
                return [dict(code='RCS001',line=1,message='message',path='file-0.py',
                             parameters=dict(ordinal=0,result_path='native-selected-path'))]
        module=SimpleNamespace(_REPO_ROOT=Path('/project'),Violation=lambda *args:args)
        with tempfile.TemporaryDirectory() as temporary:
            paths=[Path(temporary)/n for n in ['a.py','b.py']]
            for p in paths: p.write_text('TimerClaim\n')
            self.assertEqual(removed_port.scan_batch(Backend(),module,paths),
                             (('native-selected-path',1,'RCS001','message'),))
        self.assertEqual(len(calls),1); self.assertEqual(len(calls[0][1]),2)


if __name__=='__main__': unittest.main()
