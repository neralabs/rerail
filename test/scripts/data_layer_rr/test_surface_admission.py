"""Surface admission requires coherent receipts and the fixed original-test gate."""
import copy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from admit_surface import audit,load
from migration import read_json,MANIFEST,require_complete


class SurfaceAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        folder=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
        cls.reporter=load(folder/'rr-surface-full-contract-final041.json.gz')
        cls.projection=load(folder/'rr-model-projection-final041.json.gz')

    def setUp(self):
        # Exercise this historical admission receipt against its recorded
        # program bytes. A live shared-library edit must not silently rewrite
        # the receipt, nor turn fixture validation into current acceptance.
        programs={**self.reporter['programs'],**self.projection['programs']}
        archived={ROOT/name:item['text'].encode() for name,item in programs.items()}
        original=Path.read_bytes
        def recorded(path):
            return archived[path] if path in archived else original(path)
        fixture=patch.object(Path,'read_bytes',recorded)
        fixture.start();self.addCleanup(fixture.stop)

    def test_all_families_validate_and_overlapping_executions_count_once(self):
        self.assertEqual(634,audit(self.reporter,self.projection)['nativeExecutions'])

    def test_other_engine_receipts_cannot_be_combined(self):
        old=load(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-surface-supplied-facts-final957.json.gz')
        with self.assertRaises(AssertionError):audit(old,self.projection)

    def test_current_policy_bytes_must_match_the_receipts(self):
        original=Path.read_bytes
        def changed(path):
            raw=original(path)
            return raw+b'\n// changed\n' if path.name=='serverplan-grammar-surface.rr' else raw
        with patch.object(Path,'read_bytes',changed),self.assertRaises(AssertionError):audit(self.reporter,self.projection)

    def test_other_rule_can_advance_after_shared_cli_receipt(self):
        original=Path.read_bytes
        def changed(path):
            raw=original(path)
            return raw+b'\n// other rule advanced\n' if path.name=='backend-type-strings.rr' else raw
        with patch.object(Path,'read_bytes',changed):
            self.assertEqual(634,audit(self.reporter,self.projection)['nativeExecutions'])

    def test_surface_shared_library_must_still_match(self):
        original=Path.read_bytes
        def changed(path):
            raw=original(path)
            return raw+b'\n// changed\n' if path.name=='text-cli.rr' else raw
        with patch.object(Path,'read_bytes',changed),self.assertRaises(AssertionError):audit(self.reporter,self.projection)

    def test_changed_shared_load_contract_requires_fresh_acceptance(self):
        original=Path.read_bytes
        def changed(path):
            raw=original(path)
            return raw+b'\n// changed load answers\n' if path.name=='project.rr' else raw
        with patch.object(Path,'read_bytes',changed),self.assertRaises(AssertionError):
            audit(self.reporter,self.projection)

    def test_fixed_original_inventory_gate_admits_surface(self):
        entry=next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='serverplan_grammar_surface')
        require_complete([entry])

    def test_original_assertions_cannot_be_dropped_from_admission(self):
        entry=copy.deepcopy(next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='serverplan_grammar_surface'))
        entry['evidence']['reports']=[]
        with self.assertRaisesRegex(ValueError,'original assertions are missing'):require_complete([entry])


if __name__=='__main__':unittest.main()
