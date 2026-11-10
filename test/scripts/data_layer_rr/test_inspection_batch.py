"""Overlapping scan roots must preserve independent occurrences in one native batch."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class InspectionBatchTests(unittest.TestCase):
    def test_overlapping_roots_keep_distinct_document_owners(self):
        from suite import Suite
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):
                calls.append((entry,files,options));return []
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'tests').mkdir();(root/'tests/a.py').write_text('inspect.getsource(f)')
            Suite.scan(Backend(),'no_source_inspection','find_violations',SimpleNamespace(TEST_DIRECTORIES=['tests']),
                       dict(root=root,directories=['tests','tests']))
        files=calls[-1][1]
        self.assertEqual(2,len(files));self.assertEqual(2,len({f['path'] for f in files}))


if __name__=='__main__':unittest.main()
