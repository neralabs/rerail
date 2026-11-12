"""A reporter invocation owns one cache for all its cases and releases it afterward."""
import os
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
import reporting


class ReporterCache(unittest.TestCase):
    def test_native_run_reuses_an_owned_cache_and_restores_the_callers_environment(self):
        seen=[]
        def observe(_args):
            path=Path(os.environ['RERAIL_TAPE_CACHE']);seen.append(path)
            self.assertTrue(path.parent.is_dir());path.write_text('owned cache')
            self.assertEqual(str(path),os.environ['RERAIL_TAPE_CACHE'])
            return 'result'
        with patch.dict(os.environ,{'RERAIL_TAPE_CACHE':'/caller/shared.sqlite'}),patch.object(reporting,'_run',observe,create=True):
            self.assertEqual(reporting.run(SimpleNamespace(command='native')),'result')
            self.assertEqual(os.environ['RERAIL_TAPE_CACHE'],'/caller/shared.sqlite')
        self.assertNotEqual(str(seen[0]),'/caller/shared.sqlite');self.assertFalse(seen[0].parent.exists())

    def test_source_run_does_not_change_cache_environment(self):
        def observe(_args):return os.environ['RERAIL_TAPE_CACHE']
        with patch.dict(os.environ,{'RERAIL_TAPE_CACHE':'/caller/shared.sqlite'}),patch.object(reporting,'_run',observe,create=True):
            self.assertEqual(reporting.run(SimpleNamespace(command='source')),'/caller/shared.sqlite')

    def test_native_failure_cleans_up_and_restores_environment(self):
        seen=[]
        def fail(_args):
            seen.append(Path(os.environ['RERAIL_TAPE_CACHE']));raise ValueError('native contract failed')
        with patch.dict(os.environ,{'RERAIL_TAPE_CACHE':'/caller/shared.sqlite'}),patch.object(reporting,'_run',fail,create=True):
            with self.assertRaisesRegex(ValueError,'native contract failed'):reporting.run(SimpleNamespace(command='native'))
            self.assertEqual(os.environ['RERAIL_TAPE_CACHE'],'/caller/shared.sqlite')
        self.assertFalse(seen[0].parent.exists())


if __name__=='__main__':unittest.main()
