"""Historical evidence must retain omissions, API uncertainty, and source identity."""
import copy
from pathlib import Path
import sys
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from evidence_census import summarize,reports
from migration import PIN

class EvidenceCensusTests(unittest.TestCase):
    def fixtures(self):
        rule=dict(check='example',batch=1,status='draft',rulePath='example.rr')
        manifest=dict(rules=[rule],engineSource=dict(cliArtifact=dict(sha256='current')))
        inventory=dict(sourcePin=PIN,files={'test.py':dict(detectors=['example'],sha256='source')},cases={'test.py::positive':{},'test.py::negative':{}})
        phases=[dict(phase=p,outcome='passed') for p in ['setup','call','teardown']]
        row=dict(oracle=False,sourcePin=PIN,mode='snapshot',engineSha256='old',policySha256={'example':'old-rule'},
            sourceFiles={'test.py':'source'},semanticFallbacks=[],missingResults=[],exitCode=0,
            cases={'test.py::positive':phases},calls=[dict(test='test.py::positive',check='example',status='passed')])
        return manifest,inventory,row

    def result(self,row):
        m,i,_=self.fixtures()
        return summarize(m,i,[(dict(path='receipt',sha256='receipt',pointer='/native'),row)])

    def test_one_positive_does_not_complete_the_rule_or_other_api(self):
        _,_,row=self.fixtures();value=self.result(row);rule=value['rules'][0]
        self.assertEqual(rule['translation'],'draft')
        self.assertEqual(rule['historicalEvidence']['snapshot']['reportedPassed'],1)
        self.assertEqual(rule['historicalEvidence']['snapshot']['pending'],['test.py::negative'])
        self.assertEqual(rule['historicalEvidence']['workspace']['reportedPassed'],0)
        self.assertEqual(value['references'][0]['engineSha256'],'old')
        self.assertEqual(value['currentEngineSha256'],'current')

    def test_missing_phase_cannot_certify_a_passing_case(self):
        _,_,row=self.fixtures();row['cases']['test.py::positive'].pop()
        self.assertEqual(self.result(row)['rules'][0]['historicalEvidence']['snapshot']['reportedPassed'],0)

    def test_changed_original_source_is_rejected(self):
        _,_,row=self.fixtures();row['sourceFiles']['test.py']='changed'
        self.assertEqual(self.result(row)['rules'][0]['historicalEvidence']['snapshot']['reportedPassed'],0)

    def test_original_semantic_fallback_cannot_supply_native_evidence(self):
        _,_,row=self.fixtures();row['semanticFallbacks']=['example:check_file']
        self.assertEqual(self.result(row)['rules'][0]['historicalEvidence']['snapshot']['reportedPassed'],0)

    def test_unspecified_api_stays_unspecified(self):
        _,_,row=self.fixtures();del row['mode'];del row['policySha256']
        value=self.result(row)['rules'][0]['historicalEvidence']
        self.assertEqual(value['unspecified']['reportedPassed'],1)
        self.assertEqual(value['snapshot']['reportedPassed'],0)

    def test_embedded_serialized_reports_are_discovered(self):
        import json
        _,_,row=self.fixtures()
        found=list(reports({'files':{'report.json':json.dumps(row)}}))
        self.assertEqual(found[0][0],'/files/report.json#json')
        self.assertEqual(found[0][1],row)

    def test_missing_recorded_detector_return_is_not_a_pass(self):
        _,_,row=self.fixtures();row['missingResults']=[dict(test='test.py::positive')]
        self.assertEqual(self.result(row)['rules'][0]['historicalEvidence']['snapshot']['reportedPassed'],0)

    def test_another_checks_native_call_does_not_certify_this_checks_execution(self):
        _,_,row=self.fixtures();row['calls'][0]['check']='other'
        self.assertEqual(self.result(row)['rules'][0]['historicalEvidence']['snapshot']['passedWithNativeCalls'],0)

    def test_duplicate_reports_do_not_inflate_the_case_count(self):
        m,i,row=self.fixtures();ref=dict(path='receipt',sha256='receipt',pointer='/native')
        value=summarize(m,i,[(ref,row),(ref,copy.deepcopy(row))])
        self.assertEqual(value['rules'][0]['historicalEvidence']['snapshot']['reportedPassed'],1)

    def test_a_positive_requires_both_complete_values(self):
        _,_,row=self.fixtures()
        row['detectorObservations']=[dict(check='example',test='test.py::positive',status='passed',
            expected=['finding'],actual=['finding'],actualSourceResult=None)]
        self.assertEqual(self.result(row)['rules'][0]['historicalEvidence']['snapshot']['casesWithMatchedPositives'],0)
