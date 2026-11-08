"""Acceptance must execute libraries from the active engine checkout."""
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
import replay
import migration


class EngineAlignmentTests(unittest.TestCase):
    def setUp(self):
        getattr(migration,'_SOURCE_COHORTS',set()).clear()

    def test_final_tip_is_a_process_snapshot_while_artifact_checks_remain_per_execution(self):
        source=dict(checkout='.',upstreamCommit='adopted')
        with patch.object(migration,'verify_engine_alignment') as audit:
            migration.verify_source_cohort(Path('/fixture'),source)
            audit.side_effect=ValueError('Final advanced')
            migration.verify_source_cohort(Path('/fixture'),source)
            self.assertEqual(audit.call_count,1)
            with self.assertRaisesRegex(ValueError,'Final advanced'):
                migration.verify_source_cohort(Path('/fixture'),dict(source,upstreamCommit='next'))
        migration._SOURCE_COHORTS.clear()
        with patch.object(migration,'verify_engine_alignment',side_effect=ValueError('Final advanced')):
            with self.assertRaisesRegex(ValueError,'Final advanced'):
                migration.verify_source_cohort(Path('/fixture'),source)

    def artifact(self,root):
        binary=root/'frozen-cli';binary.write_bytes(b'compiled modern engine')
        trees={name:'tree:'+name for name in ['native','tools/bitlex','Cargo.toml','Cargo.lock']}
        receipt=dict(engineCommit='build',upstreamCommit='modern',sourceTrees=trees,sha256=migration.sha(binary.read_bytes()))
        return binary,dict(upstreamCommit='modern',cliArtifact=receipt)

    def artifact_git(self,root,*args):
        if args[0]=='rev-parse':return 'tree:'+args[1].removeprefix('HEAD:')
        return ''

    def test_native_artifact_requires_a_recorded_build_for_the_adopted_final(self):
        with self.assertRaisesRegex(ValueError,'recorded.*artifact'):
            migration.verify_engine_artifact(Path('/unused'),dict(upstreamCommit='modern'),Path('/old-cli'))
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);binary,source=self.artifact(root)
            source['cliArtifact']['upstreamCommit']='old'
            with self.assertRaisesRegex(ValueError,'adopted Final'):
                migration.verify_engine_artifact(root,source,binary)

    def test_native_artifact_rejects_an_old_source_tree_or_changed_working_sources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);binary,source=self.artifact(root)
            source['cliArtifact']['sourceTrees']['native']='old-tree'
            with patch.object(migration,'git',self.artifact_git),self.assertRaisesRegex(ValueError,'source tree'):
                migration.verify_engine_artifact(root,source,binary)
            source['cliArtifact']['sourceTrees']['native']='tree:native'
            def dirty(root,*args):
                return 'native/changed.rs' if args[0]=='diff' else self.artifact_git(root,*args)
            with patch.object(migration,'git',dirty),self.assertRaisesRegex(ValueError,'committed'):
                migration.verify_engine_artifact(root,source,binary)

    def test_native_artifact_validates_bytes_and_invalidates_its_hash_cache_on_change(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);binary,source=self.artifact(root)
            with patch.object(migration,'git',self.artifact_git):
                migration.verify_engine_artifact(root,source,binary)
                migration.verify_engine_artifact(root,source,binary)
                binary.write_bytes(b'compiled outdated code')
                with self.assertRaisesRegex(ValueError,'binary.*recorded'):
                    migration.verify_engine_artifact(root,source,binary)

    def test_native_artifact_is_checked_before_either_execution_boundary(self):
        import suite
        for boundary in ['direct','worker']:
            with self.subTest(boundary=boundary), \
                 patch.object(migration,'read_json',return_value=dict(engineSource={})), \
                 patch.object(migration,'verify_engine_alignment'), \
                 patch.object(migration,'verify_engine_artifact',side_effect=ValueError('stale binary')), \
                 patch.object(replay.subprocess,'run') as direct,patch.object(suite.subprocess,'Popen') as worker:
                with self.assertRaisesRegex(ValueError,'stale binary'):
                    if boundary=='direct':replay.execute_native(Path('/old-cli'),Path('/policy.rr'),[],{})
                    else:suite.Suite(Path('/source'),Path('/old-cli'),[],0,1,Path('/baseline'))
                direct.assert_not_called();worker.assert_not_called()

    def test_artifact_registration_validates_before_it_changes_the_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);binary,source=self.artifact(root)
            receipt=dict(source['cliArtifact'],path=binary.relative_to(root).as_posix())
            record=root/'build.json';migration.write_json(record,receipt)
            manifest=root/migration.MANIFEST;migration.write_json(manifest,dict(engineSource=dict(upstreamCommit='modern')))
            with patch.object(migration,'git',self.artifact_git),patch.object(migration,'verify_engine_alignment'):
                migration.register_artifact(root,record)
                self.assertEqual(migration.read_json(manifest)['engineSource']['cliArtifact'],receipt)
                before=manifest.read_bytes();binary.write_bytes(b'unrecorded engine')
                with self.assertRaisesRegex(ValueError,'binary.*recorded'):
                    migration.register_artifact(root,record)
                self.assertEqual(manifest.read_bytes(),before)

    def test_engine_audit_rejects_a_retired_checkout(self):
        with self.assertRaisesRegex(ValueError, 'repository root'):
            migration.verify_engine_alignment(Path('/unused'), dict(checkout='.rerail/engine'))

    def test_engine_audit_requires_the_adopted_commit_in_history(self):
        with patch.object(migration, 'git', side_effect=migration.subprocess.CalledProcessError(1, 'git')):
            with self.assertRaisesRegex(ValueError, 'adopted Final shape'):
                migration.verify_engine_alignment(Path('/unused'), dict(checkout='.', upstreamCommit='modern'))

    def test_engine_audit_rejects_unrecorded_native_files(self):
        def git(_root, *args):
            return 'native/parallel_executor.rs' if args[0] == 'ls-files' else ''
        with patch.object(migration, 'git', git):
            with self.assertRaisesRegex(ValueError, 'Unreviewed engine changes'):
                migration.verify_engine_alignment(Path('/unused'),
                    dict(checkout='.', upstreamCommit='modern', nativeExtensions=[]))

    def test_engine_audit_rejects_newer_local_final_shape_until_merged(self):
        branch = 'refs/heads/jeremybarnes/final-rerail-shape'
        def git(_root, *args):
            if args == ('rev-parse', '--verify', branch):
                return 'new-final-shape'
            if args == ('merge-base', '--is-ancestor', 'new-final-shape', 'HEAD'):
                raise migration.subprocess.CalledProcessError(1, 'git')
            return ''
        with patch.object(migration, 'git', git):
            with self.assertRaisesRegex(ValueError, 'newer local Final shape'):
                migration.verify_engine_alignment(Path('/unused'), dict(
                    checkout='.', upstreamCommit='adopted', upstreamBranch=branch,
                    nativeExtensions=[]))

    def test_engine_audit_allows_an_unavailable_local_final_shape_branch(self):
        branch = 'refs/heads/jeremybarnes/final-rerail-shape'
        def git(_root, *args):
            if args == ('rev-parse', '--verify', branch):
                raise migration.subprocess.CalledProcessError(128, 'git')
            return ''
        with patch.object(migration, 'git', git):
            migration.verify_engine_alignment(Path('/unused'), dict(
                checkout='.', upstreamCommit='adopted', upstreamBranch=branch,
                nativeExtensions=[]))

    def test_engine_audit_rejects_a_newer_remote_tip_even_when_local_is_merged(self):
        branch = 'refs/heads/jeremybarnes/final-rerail-shape'
        remote = 'refs/remotes/origin/jeremybarnes/final-rerail-shape'
        def git(_root, *args):
            if args == ('rev-parse', '--verify', branch):
                return 'adopted'
            if args == ('rev-parse', '--verify', remote):
                return 'new-remote-final-shape'
            if args == ('merge-base', '--is-ancestor', 'new-remote-final-shape', 'HEAD'):
                raise migration.subprocess.CalledProcessError(1, 'git')
            return ''
        with patch.object(migration, 'git', git):
            with self.assertRaisesRegex(ValueError, 'newer.*Final shape'):
                migration.verify_engine_alignment(Path('/unused'), dict(
                    checkout='.', upstreamCommit='adopted', upstreamBranch=branch,
                    nativeExtensions=[]))

    def test_native_acceptance_checks_alignment_before_starting_its_worker(self):
        import suite
        manifest = dict(engineSource=dict(checkout='.'))
        with patch.object(migration, 'read_json', return_value=manifest), \
                patch.object(migration, 'verify_engine_alignment', side_effect=ValueError('outdated engine')) as audit, \
                patch.object(suite.subprocess, 'Popen') as worker:
            with self.assertRaisesRegex(ValueError, 'outdated engine'):
                suite.Suite(Path('/source'), Path('/engine'), [], 0, 1, Path('/baseline'))
            audit.assert_called_once_with(migration.ROOT, manifest['engineSource'])
            worker.assert_not_called()

    def test_native_acceptance_rejects_a_binary_in_either_retired_engine(self):
        import suite
        for retired in ['.rerail/rr-migration-20261106/engine',
                        '.rerail/data-layer-batch-seven/engine-source']:
            with self.subTest(retired=retired), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                binary = root / retired / 'target/debug/rerail-engine'
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b'identical bytes do not make this the active checkout')
                with patch.object(suite, 'ROOT', root), \
                        patch.object(migration, 'read_json', return_value=dict(engineSource={})), \
                        patch.object(migration, 'verify_engine_alignment'), \
                        patch.object(suite.subprocess, 'Popen') as worker:
                    with self.assertRaisesRegex(ValueError, 'retired engine'):
                        suite.Suite(Path('/source'), binary, [], 0, 1, Path('/baseline'))
                    worker.assert_not_called()

    def test_engine_audit_checks_ancestry_and_exact_reviewed_extensions(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            name = 'native/src/code_rule_dsl/facts/vtab/syntax/python_print.rs'
            path = root / name
            path.parent.mkdir(parents=True)
            path.write_text('reviewed generic capability\n')
            receipt = dict(checkout='.', upstreamCommit='modern', nativeExtensions=[
                dict(path=name, sha256=replay.sha(path.read_bytes()))])
            def git(_root, *args):
                if args == ('merge-base', '--is-ancestor', 'modern', 'HEAD'): return ''
                if args == ('diff', '--name-only', 'modern', '--', 'native', 'Cargo.toml', 'Cargo.lock'):
                    return name
                if args == ('ls-files', '--others', '--exclude-standard', '--', 'native'): return ''
                raise AssertionError(args)
            with patch.object(migration, 'git', git):
                migration.verify_engine_alignment(root, receipt)
                path.write_text('an unreviewed replacement\n')
                with self.assertRaisesRegex(ValueError, 'reviewed'):
                    migration.verify_engine_alignment(root, receipt)

    def test_engine_audit_rejects_a_parallel_executor_even_if_it_is_allowlisted(self):
        receipt = dict(checkout='.', upstreamCommit='modern', nativeExtensions=[
            dict(path='native/src/code_rule_dsl/sql_executor.rs', sha256='irrelevant')])
        with patch.object(migration, 'git', return_value='native/src/code_rule_dsl/sql_executor.rs'):
            with self.assertRaisesRegex(ValueError, 'extension boundary'):
                migration.verify_engine_alignment(Path('/unused'), receipt)

    def test_identical_library_in_retired_checkout_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            active = root / 'docs/rule-syntax/lib/core.rr'
            retired = root / '.rerail/retired-engine/docs/rule-syntax/lib/core.rr'
            for path in (active, retired):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('// identical library bytes\n')
            digest = replay.sha(active.read_bytes())
            with patch.object(replay, 'ROOT', root):
                self.assertEqual([dict(path='docs/rule-syntax/lib/core.rr', sha256=digest)],
                                 replay.program_inputs([dict(path=str(active), sha256=digest)]))
                with self.assertRaisesRegex(ValueError, 'active engine'):
                    replay.program_inputs([dict(path=str(retired), sha256=digest)])

    def test_direct_native_execution_checks_alignment_before_starting_a_process(self):
        manifest=dict(engineSource=dict(checkout='.'))
        with patch.object(migration,'read_json',return_value=manifest), \
                patch.object(migration,'verify_engine_alignment',side_effect=ValueError('outdated engine')) as audit, \
                patch.object(replay.subprocess,'run') as process:
            with self.assertRaisesRegex(ValueError,'outdated engine'):
                replay.execute_native(Path('/engine'),Path('/policy.rr'),[],{})
            audit.assert_called_once_with(migration.ROOT,manifest['engineSource'])
            process.assert_not_called()

    def test_direct_native_execution_rejects_both_retired_engine_locations(self):
        for retired in ['.rerail/rr-migration-20261106/engine','.rerail/data-layer-batch-seven/engine-source']:
            with self.subTest(retired=retired), \
                    patch.object(migration,'verify_engine_alignment'), \
                    patch.object(replay.subprocess,'run') as process:
                with self.assertRaisesRegex(ValueError,'retired engine'):
                    replay.execute_native(replay.ROOT/retired/'target/debug/rerail-engine',Path('/policy.rr'),[],{})
                process.assert_not_called()
