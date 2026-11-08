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
