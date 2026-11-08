"""Workspace acceptance must exercise discovery rather than the direct scanner's inputFiles override."""
import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import migration
import replay


class WorkspaceTransportTests(unittest.TestCase):
    def execute(self,runner):
        seen={}
        files=[dict(path='impl/a.py',source='class Primary: pass\n'),
               dict(path='data_layer/api.py',source='class Facade: pass\n')]
        options=dict(marker='preserved')
        def native(command,**kwargs):
            seen['options']=json.loads(Path(command[-1]).read_text())
            corpus=Path(command[3])
            seen['files']={file['path']:(corpus/file['path']).read_text() for file in files}
            return subprocess.CompletedProcess(command,0,json.dumps(dict(diagnostics=[],programInputs=[])),'')
        with patch.object(migration,'verify_source_cohort'),patch.object(migration,'verify_engine_artifact'), \
             patch.object(replay.subprocess,'run',side_effect=native):
            runner(Path('/frozen-engine'),Path('/policy.rr'),files,options)
        self.assertEqual(options,dict(marker='preserved'))
        self.assertEqual(seen['files'],{file['path']:file['source'] for file in files})
        return seen['options']

    def test_direct_api_supplies_the_exact_document_batch(self):
        self.assertEqual(self.execute(replay.execute_native),dict(marker='preserved',inputFiles=['impl/a.py','data_layer/api.py']))

    def test_workspace_api_leaves_scope_and_loading_to_the_engine(self):
        self.assertEqual(self.execute(replay.execute_workspace),dict(marker='preserved'))

    def test_callers_cannot_smuggle_an_input_override_into_workspace_acceptance(self):
        with patch.object(replay.subprocess,'run') as native:
            with self.assertRaisesRegex(ValueError,'inputFiles'):
                replay.execute_workspace(Path('/engine'),Path('/policy.rr'),[],dict(inputFiles=['ignored.py']))
            native.assert_not_called()
