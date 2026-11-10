"""API-import scans submit the complete explicit-file batch once."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class ApiImportsBatchTests(unittest.TestCase):
    def test_one_query_for_the_complete_batch(self):
        from api_imports_port import scan_batch
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):calls.append((files,options));return []
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);paths=[root/'a.py',root/'b.py']
            for path in paths:path.write_text('from api import X\n')
            self.assertEqual(scan_batch(Backend(),SimpleNamespace(),root,paths),[])
        self.assertEqual(len(calls),1);self.assertEqual(len(calls[0][0]),2)


if __name__=='__main__':unittest.main()
