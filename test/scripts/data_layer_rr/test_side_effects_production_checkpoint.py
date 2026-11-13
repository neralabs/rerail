"""Production evidence must preserve facts, findings, scope, errors, and multiplicity."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from side_effects_production_checkpoint import validate


class SideEffectsProductionCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-side-effects-production-current.json.gz','rt')as stream:cls.receipt=json.load(stream)

    def case(self,value,name,mode='snapshot'):
        return next(row for report in value['native'][mode]for row in report['cases']if row['input']['id']==name)

    def test_complete_production_parity_passes_both_apis(self):
        self.assertEqual(validate(self.receipt),dict(productionCasesPerAPI=25,discoveryCasesPerAPI=15,positiveFileBatch=64,fullMigrationAcceptance=False))

    def test_an_omitted_case_is_rejected(self):
        value=copy.deepcopy(self.receipt);value['native']['workspace'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'Production case omitted'):validate(value)

    def test_positive_findings_cannot_disappear(self):
        value=copy.deepcopy(self.receipt);self.case(value,'positive')['actual']['findings'].clear()
        with self.assertRaisesRegex(ValueError,'Complete production observation differs'):validate(value)

    def test_absent_facts_cannot_become_empty_fact_lists(self):
        value=copy.deepcopy(self.receipt);self.case(value,'clean')['actual']['facts']['impl/a.py']['plan_scope_side_effects.violations']=[]
        with self.assertRaisesRegex(ValueError,'Complete production observation differs'):validate(value)

    def test_nonblocking_audit_facts_cannot_disappear(self):
        value=copy.deepcopy(self.receipt);self.case(value,'audit')['actual']['facts']['impl/a.py']['plan_scope_side_effects.violations'].clear()
        with self.assertRaisesRegex(ValueError,'Complete production observation differs'):validate(value)

    def test_broken_link_errors_cannot_be_silently_ignored(self):
        value=copy.deepcopy(self.receipt);self.case(value,'broken_link')['actual']={'findings':[]}
        with self.assertRaisesRegex(ValueError,'Complete production observation differs'):validate(value)

    def test_same_line_findings_cannot_be_deduplicated_in_the_public_engine(self):
        value=copy.deepcopy(self.receipt);self.case(value,'same_line')['native'][1]['diagnostics'].pop()
        with self.assertRaisesRegex(ValueError,'Same-line multiplicity omitted'):validate(value)

    def test_whole_positive_file_batch_cannot_be_split(self):
        value=copy.deepcopy(self.receipt);self.case(value,'batch')['native'].append(copy.deepcopy(self.case(value,'batch')['native'][0]))
        with self.assertRaisesRegex(ValueError,'Whole production batch was split'):validate(value)

    def test_shared_root_definition_must_be_retained(self):
        value=copy.deepcopy(self.receipt);value['snapshots']=[row for row in value['snapshots']if row['path']!='docs/rule-syntax/lib/data-layer/side-effects-production-roots.rr']
        with self.assertRaisesRegex(ValueError,'Consumed production bytes omitted'):validate(value)

    def test_progress_cannot_claim_full_admission(self):
        value=copy.deepcopy(self.receipt);value['fullMigrationAcceptance']=True
        with self.assertRaisesRegex(ValueError,'not full rule admission'):validate(value)


if __name__=='__main__':unittest.main()
