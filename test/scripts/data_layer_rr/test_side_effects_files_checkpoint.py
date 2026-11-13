"""Filesystem receipts must retain every observation and the whole positive demand batch."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from side_effects_files_checkpoint import validate


class SideEffectsFilesCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-side-effects-files-current.json.gz','rt')as f:cls.receipt=json.load(f)

    def case(self,value,name,mode='snapshot'):
        return next(r for report in value['contracts']['files'][mode]for r in report['cases']if r['input']['id']==name)

    def test_all_source_observations_pass_both_apis(self):
        self.assertEqual(validate(self.receipt),dict(filesystemCasesPerAPI=47,scannerCasesPerAPI=35,originalAssertionsPerAPI=7,positiveFileBatch=64,fullMigrationAcceptance=False))

    def test_an_omitted_case_is_rejected(self):
        value=copy.deepcopy(self.receipt);value['contracts']['files']['workspace'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'Native filesystem case omitted'):validate(value)

    def test_directory_order_cannot_be_replaced_by_set_equality(self):
        value=copy.deepcopy(self.receipt);self.case(value,'iter-ordering')['actual']['value'].reverse()
        with self.assertRaisesRegex(ValueError,'Complete filesystem observation differs'):validate(value)

    def test_external_file_link_positives_cannot_disappear(self):
        value=copy.deepcopy(self.receipt);self.case(value,'repo-external-link')['actual']['value'].clear()
        with self.assertRaisesRegex(ValueError,'Complete filesystem observation differs'):validate(value)

    def test_decode_error_precedence_cannot_change(self):
        value=copy.deepcopy(self.receipt);self.case(value,'explicit-error-order')['actual']['error']['args']['items'][1]['hex']='ff'
        with self.assertRaisesRegex(ValueError,'Complete filesystem observation differs'):validate(value)

    def test_positive_file_batch_cannot_be_split_into_callbacks(self):
        value=copy.deepcopy(self.receipt);self.case(value,'explicit-batch')['native']*=2
        with self.assertRaisesRegex(ValueError,'Whole positive file batch was split'):validate(value)

    def test_consumed_order_library_cannot_be_omitted(self):
        value=copy.deepcopy(self.receipt);value['snapshots']=[r for r in value['snapshots']if r['path']!='docs/rule-syntax/lib/data-layer/side-effects-files.rr']
        with self.assertRaisesRegex(ValueError,'Consumed bytes omitted'):validate(value)

    def test_progress_cannot_claim_full_rule_admission(self):
        value=copy.deepcopy(self.receipt);value['fullMigrationAcceptance']=True
        with self.assertRaisesRegex(ValueError,'not full rule admission'):validate(value)

    def test_final_apis_cannot_mix_different_rule_revisions(self):
        value=copy.deepcopy(self.receipt)
        call=self.case(value,'relative-repo-mapped')['native'][0]
        item=next(r for r in call['programInputs']if r['path']=='docs/rule-syntax/lib/data-layer/side-effects-files.rr')
        item['sha256']='f'*64
        with self.assertRaisesRegex(ValueError,'Final program revisions differ'):validate(value)


if __name__=='__main__':unittest.main()
