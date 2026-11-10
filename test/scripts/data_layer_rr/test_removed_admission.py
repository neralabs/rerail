"""RemovedCompat admission preserves positive coverage and API/production distinctions."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from removed_admission import validate
from migration import read_json,MANIFEST,require_complete


class RemovedAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-removed-complete-finalec3.json.gz','rt') as stream:
            cls.value=json.load(stream)

    def test_complete_recorded_contract_passes(self):
        result=validate(self.value)
        self.assertEqual(239,result['publicCasesPerAPI'])
        self.assertEqual(4,result['originalCasesPerAPI'])
        self.assertEqual(19,result['productionCases'])
        self.assertEqual(8,result['controls'])

    def test_omitted_positive_is_rejected(self):
        value=copy.deepcopy(self.value)
        part=next(r for r in value['publicReports'] if any(c['input']['id']=='TimerClaim:async' for c in r['cases']))
        part['cases']=[r for r in part['cases'] if r['input']['id']!='TimerClaim:async']
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(value)

    def test_native_api_trigger_gate_is_rejected(self):
        value=copy.deepcopy(self.value)
        row=next(c for r in value['publicReports'] for c in r['cases'] if c['input']['id']=='nfkc-trigger-off')
        row['actual']['value']=[]
        with self.assertRaisesRegex(ValueError,'Public result'):validate(value)

    def test_production_must_still_filter_by_raw_trigger(self):
        value=copy.deepcopy(self.value)
        value['productionSupplement']['report']['cases'][0]['actual']['findings']=[{}]
        with self.assertRaisesRegex(ValueError,'Production trigger result'):validate(value)

    def test_mixed_engine_is_rejected(self):
        value=copy.deepcopy(self.value);value['publicReports'][0]['engineSha256']='0'*64
        with self.assertRaisesRegex(ValueError,'Mixed engine'):validate(value)

    def test_missing_control_effect_is_rejected(self):
        value=copy.deepcopy(self.value)
        row=next(r for r in value['controls']['cases'] if r['name']=='api-trigger-bypass')
        row['disabled']['diagnostics']=row['original']['diagnostics']
        with self.assertRaisesRegex(ValueError,'undetected'):validate(value)

    def test_fixed_original_inventory_admits_rule(self):
        entry=next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='removed_compat_surfaces')
        require_complete([entry])


if __name__=='__main__':unittest.main()
