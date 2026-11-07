"""Acceptance must execute libraries from the active engine checkout."""
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
import replay


class EngineAlignmentTests(unittest.TestCase):
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
