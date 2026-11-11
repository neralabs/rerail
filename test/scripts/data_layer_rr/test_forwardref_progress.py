"""Partial scanner coverage must remain explicit and preserve complete observations."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from forwardref_progress import validate


class Progress(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-forwardref-scanner-progress-finalfcce.json.gz','rt') as stream:cls.value=json.load(stream)

    def rejected(self,value):
        with self.assertRaises(ValueError):validate(value)

    def test_progress_scope(self):
        summary=validate(self.value)
        self.assertFalse(summary['admitted'])
        self.assertEqual(summary['nativeLinterOriginalCasesPerAPI'],5)
        self.assertEqual(summary['hostWorkflowCasesPerAPI'],48)
        self.assertEqual(summary['publicCasesPerAPI'],80)
        self.assertEqual(summary['casesWithoutNativeCalls']['public']['snapshot'],['file-missing','file-invalid-utf8','content-missing'])

    def test_cannot_claim_admission(self):
        value=copy.deepcopy(self.value);value['admitted']=True;self.rejected(value)

    def test_cannot_omit_positive(self):
        value=copy.deepcopy(self.value);value['originals']['snapshot']['detectorObservations'].pop(0);self.rejected(value)

    def test_cannot_hide_host_workflow(self):
        value=copy.deepcopy(self.value);value['originals']['workspace']['casesWithoutNativeCalls']=[];self.rejected(value)

    def test_cannot_omit_public_case(self):
        value=copy.deepcopy(self.value);value['reports']['public']['snapshot'][0]['cases'].pop();self.rejected(value)

    def test_cannot_change_return(self):
        value=copy.deepcopy(self.value);value['reports']['production']['workspace'][0]['cases'][0]['actual']={};self.rejected(value)

    def test_cannot_mix_engine(self):
        value=copy.deepcopy(self.value);value['reports']['public']['snapshot'][0]['engineSha256']='different';self.rejected(value)

    def test_cannot_change_program(self):
        value=copy.deepcopy(self.value);next(p for p in value['programs'].values() if p['text'])['text']+='\n';self.rejected(value)


if __name__=='__main__':unittest.main()
