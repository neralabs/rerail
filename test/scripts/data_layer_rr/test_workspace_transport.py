"""Workspace acceptance must exercise discovery rather than the direct scanner's inputFiles override."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import migration
import replay


class WorkspaceTransportTests(unittest.TestCase):
    def test_physical_fixture_keeps_empty_directories_and_directory_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            replay.write_fixture(root,[dict(path='empty',directory=True),
                                       dict(path='pkg/a.py',source='def actual(): pass\n'),
                                       dict(path='alias',symlink='pkg'),dict(path='pkg/back',symlink='..')])
            self.assertEqual(list((root/'empty').iterdir()),[])
            self.assertTrue((root/'alias').is_symlink())
            self.assertTrue((root/'pkg/back').is_symlink())
            self.assertEqual((root/'alias/a.py').read_text(),'def actual(): pass\n')

    def test_physical_fixture_rejects_paths_and_links_outside_its_workspace(self):
        for item in [dict(path='../outside.py',source=''),dict(path='/outside.py',source=''),
                     dict(path='link',symlink='../outside'),dict(path='link',symlink='/outside')]:
            with self.subTest(item=item),tempfile.TemporaryDirectory() as temporary:
                with self.assertRaises(ValueError):replay.write_fixture(Path(temporary),[item])

    def execute(self,runner,options=None):
        seen={}
        files=[dict(path='impl/a.py',source='class Primary: pass\n'),
               dict(path='data_layer/api.py',source='class Facade: pass\n')]
        options=dict(marker='preserved') if options is None else options
        original=json.loads(json.dumps(options))
        def native(command,**kwargs):
            seen['options']=json.loads(Path(command[-1]).read_text())
            corpus=Path(command[3])
            seen['files']={file['path']:(corpus/file['path']).read_text() for file in files}
            return subprocess.CompletedProcess(command,0,json.dumps(dict(diagnostics=[],programInputs=[])),'')
        with patch.object(migration,'verify_source_cohort'),patch.object(migration,'verify_engine_artifact'), \
             patch.object(replay.subprocess,'run',side_effect=native):
            runner(Path('/frozen-engine'),Path('/policy.rr'),files,options)
        self.assertEqual(options,original)
        self.assertEqual(seen['files'],{file['path']:file['source'] for file in files})
        return seen['options']

    def test_direct_api_supplies_the_exact_document_batch(self):
        self.assertEqual(self.execute(replay.execute_native),dict(marker='preserved',inputFiles=['impl/a.py','data_layer/api.py']))

    def test_workspace_api_leaves_scope_and_loading_to_the_engine(self):
        self.assertEqual(self.execute(replay.execute_workspace),dict(marker='preserved'))

    def test_named_api_marks_the_preload_phase_and_preserves_caller_environment(self):
        options=dict(entryPoint='check_file',env=['caller=value'])
        actual=self.execute(replay.execute_workspace,options)
        self.assertEqual(actual,dict(entryPoint='check_file',env=['caller=value','data-layer.api=true']))

    def test_production_contract_keeps_native_discovery_enabled(self):
        options=dict(entryPoint='_production_contract',env=['caller=value'])
        self.assertEqual(self.execute(replay.execute_workspace,options),options)

    def test_callers_cannot_smuggle_an_input_override_into_workspace_acceptance(self):
        with patch.object(replay.subprocess,'run') as native:
            with self.assertRaisesRegex(ValueError,'inputFiles'):
                replay.execute_workspace(Path('/engine'),Path('/policy.rr'),[],dict(inputFiles=['ignored.py']))
            native.assert_not_called()

class ExistingRepositoryTransportTests(unittest.TestCase):
    def test_native_repository_keeps_binary_files_links_and_empty_primary_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'broken.py').write_bytes(b'\xff');(root/'alias.py').symlink_to('broken.py')
            for mode in ('snapshot','workspace'):
                seen={}
                def native(command,**kwargs):
                    seen['root']=Path(command[3]);seen['options']=json.loads(Path(command[-1]).read_text())
                    self.assertEqual((seen['root']/'broken.py').read_bytes(),b'\xff')
                    self.assertTrue((seen['root']/'alias.py').is_symlink())
                    return subprocess.CompletedProcess(command,0,json.dumps(dict(diagnostics=[],programInputs=[])),'')
                with patch.object(migration,'verify_source_cohort'),patch.object(migration,'verify_engine_artifact'),patch.object(replay.subprocess,'run',side_effect=native):
                    replay.execute_repository(Path('/engine'),Path('/rule.rr'),root,dict(entryPoint='find_violations'),mode)
                self.assertEqual(seen['root'],root)
                self.assertEqual(seen['options'].get('inputFiles'),[] if mode=='snapshot' else None)
