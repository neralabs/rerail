"""A generated-index checkpoint cannot hide positives or weaken native controls."""
import copy
import gzip
import json
from pathlib import Path
import sys
import unittest
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from generated_indexes_checkpoint import validate

class GeneratedIndexesCheckpointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with gzip.open(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-generated-indexes-progress-final2911.json.gz','rt') as f:cls.receipt=json.load(f)

    def example(self):return copy.deepcopy(self.receipt)

    def test_recorded_complete_progress_is_accepted_without_admitting_the_rule(self):
        self.assertFalse(validate(self.receipt)['admitted'])

    def test_missing_original_positive_is_rejected(self):
        value=self.example();row=value['originals']['workspace'];identity=next(r['test'] for r in value['originalSource']['events'] if r['findings'])
        del row['cases'][identity]
        with self.assertRaisesRegex(ValueError,'Original assertions omitted'):validate(value)

    def test_another_original_engine_is_rejected(self):
        value=self.example();value['originals']['snapshot']['engineSha256']='other'
        with self.assertRaisesRegex(ValueError,'Original native suite failed'):validate(value)

    def test_missing_original_native_execution_is_rejected(self):
        value=self.example();value['originals']['snapshot']['calls'].pop()
        with self.assertRaisesRegex(ValueError,'Original native executions'):validate(value)

    def test_omitted_supplemental_shard_is_rejected(self):
        value=self.example();value['public']['workspace'].pop()
        with self.assertRaisesRegex(ValueError,'shards omitted'):validate(value)

    def test_weakened_positive_return_is_rejected(self):
        value=self.example()
        row=next(c for part in value['public']['snapshot'] for c in part['cases'] if isinstance(c['expected'],list) and c['expected'])
        row['actual']=[]
        with self.assertRaisesRegex(ValueError,'complete return differs'):validate(value)

    def test_invalid_mutant_error_cannot_substitute_for_semantic_disagreement(self):
        value=self.example();value['controls']['cases'][0]['actual']={'error':'TypeError'}
        with self.assertRaisesRegex(ValueError,'Ineffective removal control'):validate(value)

    def test_missing_api_control_is_rejected(self):
        value=self.example();value['controls']['cases'].pop()
        with self.assertRaisesRegex(ValueError,'controls omitted'):validate(value)

    def test_missing_identifier_regression_failure_is_rejected(self):
        value=self.example();value['identifierRegression']['report']['cases'].pop()
        with self.assertRaisesRegex(ValueError,'regression failures missing'):validate(value)

    def test_identifier_red_policy_must_match_the_consumed_bytes(self):
        value=self.example();value['identifierRegression']['policy']['text']+='\\n'
        with self.assertRaisesRegex(ValueError,'regression policy differs'):validate(value)

    def test_historical_regression_cannot_be_relabelled_to_another_engine(self):
        value=self.example();value['identifierRegression']['engineSource']={'cliArtifact':{'sha256':'other'}}
        with self.assertRaisesRegex(ValueError,'regression engine differs'):validate(value)
