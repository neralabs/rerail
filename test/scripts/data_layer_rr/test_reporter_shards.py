"""Reporter shards must compare against complete, globally indexed source oracles."""
import json
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
import reporting
from migration import PIN, sha


class ReporterShardTests(unittest.TestCase):
    def test_native_partition_uses_global_oracle_indices(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cases = root / 'cases.json'
            cases.write_text(json.dumps({'cases': [dict(id=str(i), check='error_handling', files=[]) for i in range(6)]}))
            oracle = root / 'oracle.json'
            oracle.write_text(json.dumps(dict(sourcePin=PIN, execution='offline-source',
                casesSha256=sha(cases.read_bytes()), results=[dict(id=str(i), status='passed', value=[]) for i in range(6)])))
            engine = root / 'engine'
            engine.write_text('test executable')
            native = dict(metadata=dict(policies=[dict(id='data-layer-error-handling')], settings=[['mode', 'block']]),
                          diagnostics=[], programInputs=[], phases=[])
            args = SimpleNamespace(command='native', cases=cases, oracle=oracle, engine=engine,
                                   rule_root=root, part=1, parts=2)
            with patch.object(reporting, 'execute_native', return_value=native) as execute:
                result = reporting.run(args)
            self.assertEqual([1, 3, 5], result['selectedCaseIndices'])
            self.assertEqual(['1', '3', '5'], [row['id'] for row in result['results']])
            self.assertEqual(6, result['totalCases'])
            self.assertEqual(3, execute.call_count)

    def test_source_oracle_must_cover_the_complete_corpus(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'cases.json'
            path.write_text(json.dumps({'cases': [dict(id=str(i)) for i in range(4)]}))
            with self.assertRaisesRegex(ValueError, 'complete reporter source oracle'):
                reporting.run(SimpleNamespace(command='source', cases=path, part=1, parts=2))
