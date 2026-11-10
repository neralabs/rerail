"""Complete bypass admission must reject omitted assertions and alias false positives."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from blanket_admission import validate


class BlanketAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-blanket-complete-finalec3.json.gz','rt') as stream:cls.value=json.load(stream)

    def test_complete_corpus_and_controls_validate(self):
        r=validate(self.value)
        self.assertEqual((119,31,192,102,16,14,16,1635),tuple(r[k] for k in [
            'originalCasesPerAPI','originalFindingPositiveCases','publicCasesPerAPI','productionCases',
            'suppliedFactCases','persistentSteps','controls','nativeExecutions']))

    def test_fixed_original_inventory_gate_accepts_the_rule(self):
        from migration import MANIFEST,read_json,require_complete
        require_complete([next(r for r in read_json(ROOT/MANIFEST)['rules'] if r['check']=='no_blanket_grammar_bypass')])

    def test_missing_original_assertion_is_rejected(self):
        v=copy.deepcopy(self.value);v['originalSuites']['snapshot']['cases'].popitem()
        with self.assertRaisesRegex(ValueError,'Original assertions omitted'):validate(v)

    def test_source_and_disk_phase_order_cannot_be_reversed(self):
        v=copy.deepcopy(self.value);c=next(c for r in v['publicReports'] for c in r['cases'] if c['input']['id']=='content-phase-order-0')
        c['actual']['value'].reverse()
        with self.assertRaisesRegex(ValueError,'Public result'):validate(v)

    def test_alias_false_positive_cannot_be_admitted(self):
        v=copy.deepcopy(self.value);c=next(c for c in v['productionSupplement']['report']['cases'] if c['input']['id'].endswith(':alias-from-sql'))
        extra=dict(c['actual']['findings'][0],path='$ROOT/impl/common/l2200_sql_dialect/alias.py');c['actual']['findings'].append(extra)
        with self.assertRaisesRegex(ValueError,'Production trigger result differs'):validate(v)

    def test_cli_cannot_add_per_file_queries(self):
        v=copy.deepcopy(self.value);c=next(c for r in v['publicReports'] for c in r['cases'] if c['input']['kind']=='cli' and c['input']['group']=='both')
        c['native'].append(copy.deepcopy(c['native'][0]))
        with self.assertRaisesRegex(ValueError,'Bypass CLI batch execution count differs'):validate(v)

    def test_missing_file_protocol_keeps_its_complete_string_result(self):
        v=copy.deepcopy(self.value);c=next(c for c in v['originalSuites']['workspace']['detectorObservations'] if c.get('comparisonKind')=='source-value')
        c['actualSourceResult']=[]
        with self.assertRaisesRegex(ValueError,'Original DTO fields differ'):validate(v)

    def test_alias_control_cannot_be_omitted(self):
        v=copy.deepcopy(self.value);v['controls']['cases']=[c for c in v['controls']['cases'] if c['name']!='alias-scope']
        with self.assertRaisesRegex(ValueError,'Behavior controls omitted'):validate(v)
