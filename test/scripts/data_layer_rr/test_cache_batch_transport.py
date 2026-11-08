"""Native path selection must precede source reads and remain one batch for the candidate inventory."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from module_caches_port import select, for_files


class CacheBatchTransportTests(unittest.TestCase):
    def test_excluded_files_are_not_read_and_selected_sources_are_submitted_together(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for name in ['keep.py','drop.py']:
                path=root/'api'/name
                path.parent.mkdir(exist_ok=True)
                path.write_text('cache = {}\n')
            suite=SimpleNamespace(native=Mock(side_effect=[[
                dict(code='MC_FILE',path='api/keep.py',parameters=dict(group=0,i=1)),
                dict(code='MC_FILE_COUNT',path='',parameters=dict(value=1))],[]]))
            with patch.object(Path,'read_text',side_effect=AssertionError('selection opened source content')):
                pairs=select(suite,root,['api'],False)
            self.assertEqual([(root/'api/keep.py','api/keep.py')],pairs)
            with patch.object(Path,'read_text',return_value='cache = {}\n') as read:
                self.assertEqual([],for_files(suite,None,root,pairs))
            self.assertEqual(1,read.call_count)
            self.assertEqual(2,suite.native.call_count)
            first,second=suite.native.call_args_list
            self.assertEqual([],first.args[2])
            self.assertEqual(['api/drop.py','api/keep.py'],first.args[3]['candidatePaths'])
            self.assertEqual([dict(path='l0000_transport/file-0.py',source='cache = {}\n')],second.args[2])
