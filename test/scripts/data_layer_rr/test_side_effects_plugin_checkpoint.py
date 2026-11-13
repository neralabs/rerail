"""Full observations, primitive types and consumed programs are required evidence."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from side_effects_plugin_checkpoint import validate


class SideEffectsPluginCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-side-effects-plugin-current.json.gz','rt')as f:cls.receipt=json.load(f)

    def case(self,value,name,mode='snapshot'):
        return next(row for report in value['contracts']['plugin'][mode]for row in report['cases']if row['input']['id']==name)

    def test_all_source_observations_pass_through_both_apis(self):
        self.assertEqual(validate(self.receipt),dict(pluginCasesPerAPI=97,valueCasesPerAPI=7,demandCasesPerAPI=3,originalAssertionsPerAPI=7,fullMigrationAcceptance=False))

    def test_a_missing_case_is_rejected(self):
        value=copy.deepcopy(self.receipt);value['contracts']['plugin']['workspace'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'Native case omitted'):validate(value)

    def test_a_dropped_duplicate_positive_is_rejected(self):
        value=copy.deepcopy(self.receipt);self.case(value,'duplicates')['actual']['value'].pop()
        with self.assertRaisesRegex(ValueError,'Complete native observation differs'):validate(value)

    def test_bool_line_cannot_be_replaced_with_an_equal_integer(self):
        value=copy.deepcopy(self.receipt);self.case(value,'line_number-true')['actual']['value'][0]['fields']['line']=1
        with self.assertRaisesRegex(ValueError,'Complete native observation differs'):validate(value)

    def test_a_duplicate_native_case_is_rejected(self):
        value=copy.deepcopy(self.receipt);report=value['contracts']['plugin']['snapshot'][0];report['cases'].append(copy.deepcopy(report['cases'][0]))
        with self.assertRaisesRegex(ValueError,'Duplicate native case'):validate(value)

    def test_consumed_value_library_bytes_cannot_be_dropped(self):
        value=copy.deepcopy(self.receipt);value['snapshots']=[r for r in value['snapshots']if r['path']!='docs/rule-syntax/lib/python/values.rr']
        with self.assertRaisesRegex(ValueError,'Consumed bytes omitted'):validate(value)

    def test_plugin_progress_cannot_claim_full_admission(self):
        value=copy.deepcopy(self.receipt);value['fullMigrationAcceptance']=True
        with self.assertRaisesRegex(ValueError,'not full rule admission'):validate(value)

    def test_positive_batch_cannot_be_split_into_row_queries(self):
        value=copy.deepcopy(self.receipt)
        row=next(r for report in value['contracts']['demand']['snapshot']for r in report['cases']if r['input']['id']=='whole-positive-batch')
        row['native']*=2
        with self.assertRaisesRegex(ValueError,'Whole positive batch was split'):validate(value)


if __name__=='__main__':unittest.main()
