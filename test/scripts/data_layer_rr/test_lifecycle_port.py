"""Lifecycle transport cannot replace native collector ordering with line sorting."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

ROOT=Path(__file__).absolute().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class LifecyclePortTests(unittest.TestCase):
    def test_scan_sorts_by_native_collector_order(self):
        from lifecycle_port import scan_batch
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):
                calls.append((entry,files,options))
                return [dict(path='file-0.py',line=l,parameters=dict(cls=c,method='commit',
                    collector_order0=o,collector_order1=l,collector_order2=0)) for c,o,l in [('Inner',2,4),('Outer',0,6),('Outer',0,2)]]
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);path=root/'a.py';path.write_text('pass\n')
            module=SimpleNamespace(ROOT=root,Violation=lambda *args:args)
            actual=scan_batch(Backend(),module,root,[path])
            self.assertEqual([v[3] for v in actual],[2,6,4]);self.assertEqual(len(calls),1)


if __name__=='__main__':unittest.main()
