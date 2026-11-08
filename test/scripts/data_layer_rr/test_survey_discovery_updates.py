"""Persistent-workspace acceptance retains observable updates and host-input isolation."""
import gzip
import json
import os
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import survey_discovery_updates as contract


class SurveyDiscoveryUpdatesTests(unittest.TestCase):
    def recording(self):
        path=contract.ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-survey-updates-source.json.gz'
        return json.loads(gzip.decompress(path.read_bytes()))

    def test_source_sequence_contains_observable_deletion_restoration_and_edits(self):
        rows=self.recording()['cases']
        self.assertEqual([{k:r[k] for k in contract.steps()[0]} for r in rows],contract.steps())
        self.assertNotEqual(rows[0]['expected']['assemblyPaths'],rows[1]['expected']['assemblyPaths'])
        self.assertLessEqual(len(rows[4]['expected']['paths']),50)
        self.assertGreater(len(rows[5]['expected']['paths']),50)
        self.assertNotEqual(rows[5]['expected']['analysis'],rows[6]['expected']['analysis'])
        self.assertEqual(rows[6]['expected'],rows[7]['expected'])

    def test_native_host_transport_restores_environment_even_after_failure(self):
        with patch.dict(os.environ,NERA_LINTER_WORKERS='original'):
            step=contract.steps()[0]
            with self.assertRaises(RuntimeError):
                with contract.process_environment(step):
                    self.assertEqual(os.environ['NERA_LINTER_WORKERS'],'1')
                    self.assertNotIn('PYTHONINTMAXSTRDIGITS',os.environ)
                    raise RuntimeError('interrupted')
            self.assertEqual(os.environ['NERA_LINTER_WORKERS'],'original')

    def test_unexpected_native_diagnostic_cannot_pass_as_empty_analysis(self):
        with self.assertRaises(ValueError):
            contract.observed(dict(diagnostics=[dict(code='IDC_SOURCE_GRAPH_REQUIRED')]))
