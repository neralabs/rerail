"""Discovery parity requires physical file selection, positives, and complete recordings."""
import contextlib
import copy
import gzip
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import survey_discovery_contract as contract


class SurveyDiscoveryContractTests(unittest.TestCase):
    def recording(self):
        return json.loads(gzip.decompress((contract.ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-survey-discovery-source.json.gz').read_bytes()))

    def test_guards_require_links_duplicates_component_order_and_findings(self):
        original=self.recording();contract.check_positives(original['cases'])
        for label in ['nested-directory-link-pruned','explicit-directory-link-root','explicit-repeated-root',
                      'public-expansion-deduplicates','facade-resource-outside-scan','component-path-order','dot-root-prunes','weighted-worker-order']:
            with self.subTest(label=label):
                value=copy.deepcopy(original)
                item=next(c for c in value['cases'] if c['id']==label)
                item['expected']['paths']=[]
                with self.assertRaises(ValueError):contract.check_positives(value['cases'])

    def run_native(self,root,baseline,execute):
        source=root/'source.json';source.write_text(json.dumps(baseline))
        engine=root/'engine';engine.write_text('frozen engine identity')
        output=root/'native.json'
        command=['contract','native','--baseline',str(source),'--engine',str(engine),'--output',str(output)]
        with patch.object(sys,'argv',command),patch.object(contract,'native_case',side_effect=execute), \
             patch.object(contract,'source_case',side_effect=AssertionError('Source discovery fallback')), \
             contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
            status=contract.main()
        return status,json.loads(output.read_text())

    def test_empty_native_results_cannot_pass_positive_discovery_and_analysis(self):
        baseline=self.recording();categories=list(baseline['cases'][0]['expected']['analysis'])
        with tempfile.TemporaryDirectory() as temporary:
            status,result=self.run_native(Path(temporary),baseline,
                lambda *args:(dict(paths=[],demandedPaths=[],assemblyPaths=[],analysis={name:[] for name in categories}),[]))
        self.assertEqual(status,1);self.assertTrue(result['complete']);self.assertFalse(result['passed'])
        self.assertEqual(len(result['cases']),len(contract.CASES))
        first=result['cases'][0]
        self.assertNotEqual(first['actual']['paths'],first['expected']['paths'])
        self.assertNotEqual(first['actual']['analysis'],first['expected']['analysis'])

    def test_modified_scope_links_or_incomplete_recordings_are_rejected(self):
        for change in ['missing','directories','link','incomplete']:
            with self.subTest(change=change),tempfile.TemporaryDirectory() as temporary:
                value=self.recording()
                if change=='missing':value['cases'].pop()
                if change=='directories':value['cases'][0]['directories']=['other']
                if change=='link':value['cases'][8]['files'][-1]['symlink']='different'
                if change=='incomplete':value['complete']=False
                with self.assertRaises(SystemExit):
                    self.run_native(Path(temporary),value,lambda *args:self.fail('Must reject before execution'))

    def test_interrupted_discovery_cannot_claim_completion(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            with self.assertRaises(TimeoutError):
                self.run_native(root,self.recording(),lambda *args:(_ for _ in ()).throw(TimeoutError('interrupted')))
            result=json.loads((root/'native.json').read_text())
        self.assertFalse(result['passed']);self.assertFalse(result['complete']);self.assertEqual(result['cases'],[])

    def test_matching_findings_cannot_hide_a_changed_assembly_order(self):
        baseline=self.recording()
        def execute(engine,rule,item):
            actual=copy.deepcopy(item['expected'])
            if item['id']=='weighted-worker-order':actual['assemblyPaths'].reverse()
            return actual,[]
        with tempfile.TemporaryDirectory() as temporary:
            status,result=self.run_native(Path(temporary),baseline,execute)
        self.assertEqual(status,1)
        self.assertEqual([c['id'] for c in result['cases'] if not c['passed']],['weighted-worker-order'])

    def test_guard_requires_full_weighted_occurrence_order(self):
        for label in ['weighted-worker-order','weighted-duplicates-and-zeros']:
            with self.subTest(label=label):
                recording=self.recording()
                case=next(c for c in recording['cases'] if c['id']==label)
                case['expected']['assemblyPaths'].pop()
                with self.assertRaises(ValueError):contract.check_positives(recording['cases'])
