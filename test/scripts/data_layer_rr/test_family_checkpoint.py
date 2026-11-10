"""A rule family cannot be admitted after losing cases, positives or native evidence."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).absolute().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from family_checkpoint import validate
from migration import MANIFEST,read_json,require_complete


class ClassVarAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path=ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-classvar-complete-finalfc6.json.gz'
        cls.value=json.loads(gzip.decompress(path.read_bytes()))

    def changed(self):return copy.deepcopy(self.value)

    def test_complete_family_passes(self):
        result=validate(self.value)
        self.assertEqual(result['casesPerAPI'],dict(public=82,components=36,cli=40,production=17,original=8,positive=5,updates=19,controls=9))
        self.assertEqual(result['nativeExecutions'],1198)

    def test_missing_case_is_rejected(self):
        value=self.changed();value['groups']['public'][0]['cases'].pop()
        with self.assertRaisesRegex(ValueError,'omitted or repeated'):validate(value)

    def test_missing_positive_is_rejected(self):
        value=self.changed();case=next(c for r in value['groups']['public'] for c in r['cases'] if c['input']['id']=='comparison-get_origin(x)-is')
        case['actual']['value']=[]
        with self.assertRaisesRegex(ValueError,'Complete native result'):validate(value)

    def test_false_native_coverage_is_rejected(self):
        value=self.changed();case=next(c for r in value['groups']['public'] for c in r['cases'] if c['input']['id']=='comparison-get_origin(x)-is')
        case['native']=[]
        with self.assertRaisesRegex(ValueError,'never submitted native work'):validate(value)

    def test_mixed_engine_is_rejected(self):
        value=self.changed();value['groups']['components'][0]['engineSha256']='0'*64
        with self.assertRaisesRegex(ValueError,'Mixed engine'):validate(value)

    def test_original_skip_is_rejected(self):
        value=self.changed();phases=next(iter(value['originalSuites']['snapshot']['cases'].values()))
        next(p for p in phases if p['phase']=='call')['outcome']='skipped'
        with self.assertRaisesRegex(ValueError,'failed or skipped'):validate(value)

    def test_original_dto_change_is_rejected(self):
        value=self.changed();rows=value['originalSuites']['snapshot']['detectorObservations']
        row=next(r for r in rows if r['actualSourceResult']);row['actualSourceResult'][0]['items'][2]+=1
        with self.assertRaisesRegex(ValueError,'Original full DTO'):validate(value)

    def test_missing_update_sequence_is_rejected(self):
        value=self.changed();value['updates']['workspace'].pop()
        with self.assertRaisesRegex(ValueError,'sequence omitted'):validate(value)

    def test_ineffective_control_is_rejected(self):
        value=self.changed();row=value['controls']['snapshot']['cases'][0];row['actual']=copy.deepcopy(row['expected'])
        with self.assertRaisesRegex(ValueError,'Ineffective removal'):validate(value)

    def test_foreign_source_test_bytes_are_rejected(self):
        value=self.changed();value['originalSuites']['workspace']['sourceFiles']['scripts/tests/test_lint_classvar_check.py']='0'*64
        with self.assertRaisesRegex(ValueError,'Original test source changed'):validate(value)

    def test_fixed_inventory_admits_rule(self):
        entry=next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='classvar_check')
        require_complete([entry])
