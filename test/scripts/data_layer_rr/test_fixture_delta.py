"""Fixture updates remove physical aliases and explicit directories exactly once."""
import os
from pathlib import Path
import sys
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from fixture_delta import apply
from replay import write_fixture


class FixtureDelta(unittest.TestCase):
    def test_case_alias_removal(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);previous={p:dict(path=p,source='pass\n') for p in ['impl/a.py','impl/a.PY']};write_fixture(root,list(previous.values()))
            self.assertEqual(apply(root,previous,{}),sorted(previous))
            self.assertEqual(list(root.iterdir()),[])

    def test_explicit_parent_removal(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);previous={'pkg':dict(path='pkg',directory=True),'pkg/a.py':dict(path='pkg/a.py',source='pass\n')};write_fixture(root,list(previous.values()))
            self.assertEqual(apply(root,previous,{}),sorted(previous))
            self.assertEqual(list(root.iterdir()),[])

    def test_hardlinks_are_both_removed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'a.py').write_text('pass\n');os.link(root/'a.py',root/'b.py');previous={p:dict(path=p,source='pass\n') for p in ['a.py','b.py']}
            apply(root,previous,{})
            self.assertEqual(list(root.iterdir()),[])

    def test_untouched_entry_keeps_mtime(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);previous={p:dict(path=p,source='pass\n') for p in ['keep.py','change.py']};write_fixture(root,list(previous.values()));stamp=(root/'keep.py').stat().st_mtime_ns
            current=dict(previous);current['change.py']=dict(path='change.py',source='x=1\n')
            self.assertEqual(apply(root,previous,current),['change.py'])
            self.assertEqual((root/'keep.py').stat().st_mtime_ns,stamp)


if __name__=='__main__':unittest.main()
