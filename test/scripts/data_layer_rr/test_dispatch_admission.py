"""Dispatch admission needs both complete contracts and native production errors."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from admit_dispatch import audit


class DispatchAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
        with gzip.open(folder/'rr-dispatch-cli-finalaf8-progress.json.gz','rt') as stream:cls.contracts=json.load(stream)
        with gzip.open(folder/'rr-dispatch-inventory-finalaf8-progress.json.gz','rt') as stream:cls.inventory=json.load(stream)

    def test_complete_same_engine_receipts_pass(self):
        result=audit(self.contracts,self.inventory)
        self.assertEqual((713,49,23,7),(result['nativeExecutions'],result['originalCasesPerAPI'],result['inventoryIoCasesPerAPI'],result['controlsPerAPI']))

    def test_fixed_original_inventory_gate_accepts_dispatch(self):
        from migration import MANIFEST,read_json,require_complete
        entry=next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='serverplan_dispatch_architecture')
        require_complete([entry])

    def test_missing_original_assertion_prevents_admission(self):
        contracts=copy.deepcopy(self.contracts)
        report=next(r for name,r in contracts['reports'].items() if contracts['reportKinds'][name]==['original','snapshot'])
        report['cases'].popitem()
        with self.assertRaisesRegex(ValueError,'Original test coverage'):audit(contracts,self.inventory)

    def test_native_io_cases_cannot_be_omitted(self):
        inventory=copy.deepcopy(self.inventory);inventory['ioReports'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):audit(self.contracts,inventory)

    def test_shared_program_bytes_must_agree_across_receipts(self):
        inventory=copy.deepcopy(self.inventory)
        name='docs/rule-syntax/lib/data-layer/dispatch-repository.rr'
        from migration import sha
        rows=[r for r in inventory['programs'].values() if r['path']==name]
        self.assertTrue(rows)
        replacement=rows[0]['text']+'\n// changed\n';digest=sha(replacement.encode())
        for r in rows:r.update(text=replacement,sha256=digest)
        def change(value):
            if isinstance(value,dict):
                for entry in value.get('programInputs',[]):
                    if entry['path']==name:entry['sha256']=digest
                for child in value.values():change(child)
            elif isinstance(value,list):
                for child in value:change(child)
        change(inventory)
        for key in list(inventory['programs']):
            if key.startswith(name+'@'):
                row=inventory['programs'].pop(key);inventory['programs'][name+'@'+digest]=row
        with self.assertRaises(AssertionError):audit(self.contracts,inventory)


if __name__=='__main__':unittest.main()
