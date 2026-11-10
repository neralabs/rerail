"""The repository transport must preserve all files and keep directories out of buffers."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from dispatch_repository_contract import execute_case
from replay import write_fixture


class DispatchRepositoryTransportTests(unittest.TestCase):
    def test_snapshot_preserves_all_file_buffers_with_order_and_duplicates(self):
        files=[dict(path='outside.py',source='outside'),dict(path='tests/a.py',source='test'),
            dict(path='notes.txt',source='text'),dict(path='directory.py',directory=True),
            dict(path='file-alias.py',symlink='outside.py'),dict(path='directory-alias',symlink='tests'),
            dict(path='outside.py',source='outside')]
        def execute(engine,rule,physical,options,sources,workspace):
            self.assertEqual(physical,files)
            with tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary);write_fixture(root,physical)
                return options(root)
        with patch('replay._execute',side_effect=execute):
            actual=execute_case(Path('engine'),Path('rule'),files,dict(entryPoint='_production_contract'),'snapshot')
        self.assertEqual(actual,dict(entryPoint='_production_contract',inputFiles=['outside.py','tests/a.py','notes.txt','file-alias.py','outside.py']))

    def test_workspace_keeps_directory_fixture_entries_and_no_input_override(self):
        files=[dict(path='dir',directory=True)];options=dict(entryPoint='_production_contract')
        with patch('replay.execute_workspace',return_value='actual') as execute:
            self.assertEqual(execute_case(Path('engine'),Path('rule'),files,options,'workspace'),'actual')
        execute.assert_called_once_with(Path('engine'),Path('rule'),files,options)


if __name__=='__main__':unittest.main()
