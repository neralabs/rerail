"""A native weighted-partition comparison must retain task, occurrence, and bin identity."""
import sys
from pathlib import Path
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import weighted_partition_contract as contract


class WeightedPartitionContractTests(unittest.TestCase):
    def record(self):
        return dict(spec=dict(id='one',bins=2),rows=[dict(row=0,key='a',weight=0)],expected=[[0]])

    def diagnostic(self,bin=0,row=0,ordinal=0,task=0):
        return dict(code='BALANCED_ROW',parameters=dict(task=task,row=row,bin=bin,ordinal=ordinal))

    def test_matching_chunks_with_invalid_bin_identity_are_rejected(self):
        for bin in [-1,1,1000]:
            with self.subTest(bin=bin),self.assertRaises(ValueError):
                contract.native_results([self.record()],[self.diagnostic(bin=bin)])

    def test_occurrences_cannot_disappear_or_repeat(self):
        for rows in [[],[self.diagnostic(),self.diagnostic()], [self.diagnostic(row=1)], [self.diagnostic(ordinal=1)]]:
            with self.subTest(rows=rows),self.assertRaises(ValueError):
                contract.native_results([self.record()],rows)

    def test_valid_complete_occurrence_passes(self):
        self.assertTrue(contract.native_results([self.record()],[self.diagnostic()])[0]['passed'])
