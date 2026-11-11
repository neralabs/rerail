"""Scanner progress must preserve every source result and keep admission open."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from scanner_checkpoint import validate


class ScannerCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-conn-union-scanner-finalfcce.json.gz','rt') as stream:cls.receipt=json.load(stream)

    def example(self):return copy.deepcopy(self.receipt)

    def test_recorded_scanner_progress_does_not_admit_the_rule(self):
        summary=validate(self.receipt)
        self.assertEqual(summary['scannerCasesPerAPI'],64)
        self.assertEqual(summary['originalCasesPerAPI'],5)
        self.assertEqual(summary['originalPositiveObservationsPerAPI'],2)
        self.assertFalse(summary['admitted'])

    def test_missing_original_positive_fails(self):
        value=self.example();identity=next(e['test'] for e in value['originalSource']['events'] if e['findings'])
        del value['originals']['snapshot']['cases'][identity]
        with self.assertRaisesRegex(ValueError,'Original scanner cases omitted'):validate(value)

    def test_missing_original_observation_fails(self):
        value=self.example();value['originals']['snapshot']['detectorObservations'].pop()
        with self.assertRaisesRegex(ValueError,'Original observations omitted'):validate(value)

    def test_omitted_outside_path_positive_fails(self):
        value=self.example()
        for report in value['scanners']['workspace']:report['cases'][:]=[r for r in report['cases'] if r['input']['id']!='path-outside/a.py']
        with self.assertRaisesRegex(ValueError,'Scanner cases omitted'):validate(value)

    def test_weakened_result_fails(self):
        value=self.example();row=value['scanners']['snapshot'][0]['cases'][0];row['actual']={'value':[]}
        with self.assertRaisesRegex(ValueError,'Scanner complete result differs'):validate(value)

    def test_mixed_engine_fails(self):
        value=self.example();value['scanners']['workspace'][0]['engineSha256']='other'
        with self.assertRaisesRegex(ValueError,'Scanner engine or API failed'):validate(value)

    def test_original_fallback_fails(self):
        value=self.example();value['originals']['workspace']['semanticFallbacks']=['scanner']
        with self.assertRaisesRegex(ValueError,'omitted or delegated'):validate(value)

    def test_changed_recording_fails(self):
        value=self.example();value['source']['cases'][0]['expected']={'value':[]}
        with self.assertRaises(ValueError):validate(value)

    def test_scanner_receipt_cannot_claim_full_admission(self):
        value=self.example();value['admitted']=True
        with self.assertRaisesRegex(ValueError,'Invalid scanner progress scope'):validate(value)

    def test_tdd_receipt_proves_the_path_defects_on_the_same_engine(self):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-conn-union-scanner-red-finalfcce.json.gz','rt') as stream:red=json.load(stream)
        self.assertEqual(red['engineSource']['cliArtifact'],self.receipt['engineSource']['cliArtifact'])
        failures=[(r['executionMode'],c) for r in red['reports'] for c in r['cases'] if not c['passed']]
        self.assertEqual(len(failures),9)
        self.assertTrue(any(mode=='workspace' and case['input']['id']=='path-outside/a.py' and case['expected']['value'] and not case['actual']['value'] for mode,case in failures))
        self.assertTrue(any(case['input']['id']=='path-impl/.py' and not case['expected']['value'] and case['actual']['value'] for _,case in failures))
