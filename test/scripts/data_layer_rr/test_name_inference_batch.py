"""Name-inference default selection must retain the source iterable protocol."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class NameInferenceBatchTests(unittest.TestCase):
    def test_empty_list_uses_default_paths_in_one_batch(self):
        from suite import Suite
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):
                calls.append((entry,files,options));return []
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'a.py').write_text('column == "count"')
            module=SimpleNamespace(_SCAN_ROOTS=(root,))
            Suite.scan(Backend(),'serverplan_name_inference','violations',module,dict(paths=[]))
        scans=[c for c in calls if c[0]=='violations']
        self.assertEqual(1,len(scans));self.assertEqual(1,len(scans[0][1]))


if __name__=='__main__':unittest.main()
