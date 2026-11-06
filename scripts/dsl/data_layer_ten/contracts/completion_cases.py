# SPDX-License-Identifier: Apache-2.0
"""The published DoD matrix must reject missing, duplicate, skipped, and failed work."""
import copy
import json
import sys
from pathlib import Path
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from data_layer_rule_audit import fixtures_path
from data_layer_ten.completion import verify_contracts


@pytest.fixture
def report():
    receipt=json.loads(fixtures_path('host/data-layer-port/batch-10-native-assertions.json').read_text())
    names=receipt['dodCompletion']['assertionIds']
    assert len(names)==54
    return dict(numFailedTests=0, testResults=[dict(assertionResults=[dict(fullName=name,status='passed') for name in names])])


def test_exact_published_contract_matrix_is_accepted(report):
    assert verify_contracts([report])['assertions']==54


@pytest.mark.parametrize('fragment',['approved-one through snapshot','invalid-internal through workspace','optimizer-document-dependency','matching roots through snapshot','original signer'])
def test_each_production_contract_is_required(report,fragment):
    rows=report['testResults'][0]['assertionResults']
    row=next(row for row in rows if fragment in row['fullName'])
    rows.remove(row)
    with pytest.raises(ValueError,match='identities'):verify_contracts([report])


def test_duplicate_runs_cannot_cover_missing_work(report):
    with pytest.raises(ValueError,match='Duplicate'):verify_contracts([report,copy.deepcopy(report)])


@pytest.mark.parametrize('status',['failed','skipped','pending'])
def test_a_nonpassing_assertion_never_counts_as_coverage(report,status):
    report['testResults'][0]['assertionResults'][0]['status']=status
    with pytest.raises(ValueError,match='identities'):verify_contracts([report])


def test_a_runtime_error_never_counts_as_completion(report):
    report['numRuntimeErrorTestSuites']=1
    with pytest.raises(ValueError,match='failed'):verify_contracts([report])
