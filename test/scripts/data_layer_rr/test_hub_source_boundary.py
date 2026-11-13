"""Hub adoption must audit every engine crate, including the new forest crate."""
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import migration


class HubSourceBoundaryTests(unittest.TestCase):
    def test_snapshot_rejects_tracked_and_untracked_changes_in_forest_and_bitlex(self):
        with tempfile.TemporaryDirectory()as temporary:
            root=Path(temporary)
            migration.git(root,'init','-q')
            files=['native/src/lib.rs','accel/src/lib.rs','forest/src/lib.rs','tools/bitlex/src/lib.rs','Cargo.toml','Cargo.lock']
            for name in files:
                p=root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text('baseline\n')
            migration.git(root,'add','--',*files)
            migration.git(root,'-c','user.name=Rerail test','-c','user.email=rerail-test@example.invalid','commit','-qm','Fixture baseline')
            source=dict(checkout='.',upstreamCommit=migration.git(root,'rev-parse','HEAD'),nativeExtensions=[])
            migration.verify_engine_snapshot(root,source)
            for prefix in ['forest/src','tools/bitlex/src']:
                for tracked in [True,False]:
                    with self.subTest(crate=prefix,tracked=tracked):
                        path=root/prefix/('lib.rs'if tracked else 'unreviewed.rs')
                        path.write_text('unreviewed engine change\n')
                        try:
                            with self.assertRaisesRegex(ValueError,'Unreviewed engine changes'):
                                migration.verify_engine_snapshot(root,source)
                        finally:
                            if tracked:path.write_text('baseline\n')
                            else:path.unlink()

    def test_artifact_identity_covers_the_forest_crate(self):
        self.assertIn('forest',migration.ENGINE_SOURCE_TREES)


if __name__=='__main__':unittest.main()
