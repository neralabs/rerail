# SPDX-License-Identifier: Apache-2.0
"""Reject incomplete semantic evidence and relabelled historical measurements."""
import copy
import json
from pathlib import Path
import pytest

from data_layer_ten.common import PIN
from data_layer_ten.contracts.reaudit_evidence import digest, repository_delta, verify


@pytest.fixture
def observation():
    oracle = json.loads(Path('fixtures/host/data-layer-port/batch-10-reaudit.json').read_text())
    names = [f'{mode} preserves original production boundaries: {row["test"]}' for row in oracle['records']
             for mode in ['snapshot', 'workspace']]
    names += ['rejects a disabled configured detector in the fresh audit: ' + check
              for check in sorted({row['check'] for row in oracle['records']})]
    names += ['retains original configured positives and clean controls for all ten audited rules',
              'preserves HNSW file-to-directory scope changes in one persistent workspace']
    report = dict(numFailedTests=0, testResults=[dict(assertionResults=[dict(fullName=name, status='passed') for name in names])])
    return oracle, report


def test_exact_fresh_matrix_is_required(observation):
    oracle, report = observation
    assert verify(oracle, [report])['assertions'] == 660
    report['testResults'][0]['assertionResults'].pop()
    with pytest.raises(ValueError, match='identities'):
        verify(oracle, [report])


def test_duplicate_runs_do_not_supply_missing_comparisons(observation):
    oracle, report = observation
    with pytest.raises(ValueError, match='Duplicate'):
        verify(oracle, [report, copy.deepcopy(report)])


def test_all_rules_require_clean_and_positive_original_controls(observation):
    oracle, report = observation
    next(row for row in oracle['records'] if row['test'] == 'hnsw_architecture-control-positive')['expected'] = []
    with pytest.raises(ValueError, match='positive or clean'):
        verify(oracle, [report])


@pytest.mark.parametrize('status', ['pending', 'skipped', 'failed'])
def test_nonpassing_comparisons_are_not_coverage(observation, status):
    oracle, report = observation
    report['testResults'][0]['assertionResults'][0]['status'] = status
    with pytest.raises(ValueError, match='identities'):
        verify(oracle, [report])


@pytest.fixture
def delta(tmp_path):
    rule = 'data-layer-hnsw-architecture'
    current = digest(Path('packs/data-layer/rules') / (rule + '.json'))
    event = dict(check='hnsw_architecture', findings=[])
    recording = dict(commit=PIN, files=[dict(path='vector_types.py', source='pass\n')],
                     source_mismatches=[], events=[event])
    original, selected, native = [tmp_path / (name + '.json') for name in ['original', 'selected', 'native']]
    for path in [original, selected]:
        path.write_text(json.dumps(recording))
    historical = dict(recordingSha256=digest(original), files=1, engineSha256='engine', policyFileSha256={rule: 'old'})
    result = dict(commit=PIN, files=1, recordingSha256=digest(selected), nativeSha256='engine',
                  policyFileSha256={rule: current}, falsePositives=0, falseNegatives=0, failures=0,
                  incrementalDifferences=0, expected=0, actual=0, measuredPhases=['cold'], coldMs=1,
                  compilationMs=1, peakRssBytes=1)
    native.write_text(json.dumps(result))
    return historical, original, selected, native


def test_rule_delta_keeps_the_joint_measurement_historical(delta):
    result = repository_delta(*delta)
    assert result['revalidatedRules'] == ['data-layer-hnsw-architecture']
    assert 'separate executions' in result['note']


@pytest.mark.parametrize(('field', 'value', 'message'), [
    ('nativeSha256', 'different-engine', 'changed engine'),
    ('policyFileSha256', {}, 'every changed rule'),
    ('recordingSha256', 'different-workload', 'Changed revalidation recording'),
    ('actual', 1, 'multiplicity'), ('failures', 1, 'failed'),
])
def test_stale_or_failed_kernel_execution_cannot_refresh_a_baseline(delta, field, value, message):
    path = delta[-1]
    data = json.loads(path.read_text())
    data[field] = value
    path.write_text(json.dumps(data))
    with pytest.raises(ValueError, match=message):
        repository_delta(*delta)


def test_a_file_subset_cannot_refresh_repository_parity(delta):
    selected = delta[2]
    data = json.loads(selected.read_text())
    data['files'] = []
    selected.write_text(json.dumps(data))
    with pytest.raises(ValueError, match='file domain'):
        repository_delta(*delta)
