"""CLI acceptance compares bytes and errors, rather than finding counts alone."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from side_effects_cli_checkpoint import validate


class SideEffectsCliCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-side-effects-cli-current.json.gz','rt')as stream:cls.receipt=json.load(stream)

    def case(self,value,name,mode='snapshot'):
        return next(row for report in value['native'][mode]for row in report['cases']if row['input']['id']==name)

    def test_all_cli_observations_pass_both_apis(self):
        self.assertEqual(validate(self.receipt),dict(cliCasesPerAPI=42,fullMigrationAcceptance=False))

    def test_omitted_case_is_rejected(self):
        value=copy.deepcopy(self.receipt);value['native']['workspace'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'CLI case omitted'):validate(value)

    def test_audit_only_findings_cannot_fail_check(self):
        value=copy.deepcopy(self.receipt);self.case(value,'cli-audit-True')['actual']['value']=1
        with self.assertRaisesRegex(ValueError,'Complete CLI observation differs'):validate(value)

    def test_partial_output_before_error_cannot_disappear(self):
        value=copy.deepcopy(self.receipt);self.case(value,'report-bad-severity-True')['actual']['stdout']=''
        with self.assertRaisesRegex(ValueError,'Complete CLI observation differs'):validate(value)

    def test_supplied_order_and_duplicates_cannot_be_sorted_or_deduplicated(self):
        value=copy.deepcopy(self.receipt);row=self.case(value,'report-order-False');row['actual']['stdout']=row['actual']['stdout'].replace('a.py:1: C ERROR message\n','')
        with self.assertRaisesRegex(ValueError,'Complete CLI observation differs'):validate(value)

    def test_unicode_uppercase_cannot_be_replaced_by_ascii(self):
        value=copy.deepcopy(self.receipt);row=self.case(value,'report-unicode-False');row['actual']['stdout']=row['actual']['stdout'].replace('STRASSE ΣI','STRAßE σı')
        with self.assertRaisesRegex(ValueError,'Complete CLI observation differs'):validate(value)

    def test_help_text_cannot_be_lost(self):
        value=copy.deepcopy(self.receipt);self.case(value,'cli-parser---help')['actual']['stdout']=''
        with self.assertRaisesRegex(ValueError,'Complete CLI observation differs'):validate(value)

    def test_consumed_report_library_cannot_be_omitted(self):
        value=copy.deepcopy(self.receipt);value['snapshots']=[row for row in value['snapshots']if row['path']!='docs/rule-syntax/lib/data-layer/side-effects-cli.rr']
        with self.assertRaisesRegex(ValueError,'Consumed CLI bytes omitted'):validate(value)

    def test_progress_cannot_claim_full_admission(self):
        value=copy.deepcopy(self.receipt);value['fullMigrationAcceptance']=True
        with self.assertRaisesRegex(ValueError,'not full rule admission'):validate(value)


if __name__=='__main__':unittest.main()
