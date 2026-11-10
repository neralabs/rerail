"""Production I/O parity must come from native rows and retain positive witnesses."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from dispatch_inventory_checkpoint import validate


class DispatchInventoryCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-dispatch-inventory-finalaf8-progress.json.gz','rt') as stream:cls.value=json.load(stream)

    def test_complete_native_io_and_controls_validate(self):
        result=validate(self.value)
        self.assertEqual((23,7,62),(result['ioCasesPerAPI'],result['disabledBehaviorControlsPerAPI'],result['nativeExecutions']))
        self.assertFalse(result['admitted'])

    def test_omitted_positive_scan_is_rejected(self):
        value=copy.deepcopy(self.value)
        for report in value['ioReports']:
            if report['mode']=='snapshot':report['cases']=[r for r in report['cases'] if r['input']['id']!='scan-fifo']
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(value)

    def test_python_preflight_error_cannot_replace_native_error_rows(self):
        value=copy.deepcopy(self.value)
        row=next(r for report in value['ioReports'] for r in report['cases'] if 'error' in r['expected'])
        row['native']['diagnostics']=[r for r in row['native']['diagnostics'] if r['code']!='SDR_IO_ERROR']
        with self.assertRaisesRegex(ValueError,'native producer'):validate(value)

    def test_missing_disabled_family_control_is_rejected(self):
        value=copy.deepcopy(self.value);value['controlReports'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'Controls omitted'):validate(value)

    def test_changed_oracle_cannot_retain_its_original_hash(self):
        value=copy.deepcopy(self.value);value['source']['text']=value['source']['text'].replace('scan-fifo','invented',1)
        with self.assertRaisesRegex(ValueError,'Source bytes'):validate(value)


if __name__=='__main__':unittest.main()
