# SPDX-License-Identifier: Apache-2.0
"""Completion requires original unit assertions to execute their native contracts."""
import copy
import sys
from pathlib import Path
import pytest
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from data_layer_ten.assertions.evidence import verify_helpers


def example():
    case = 'helper-case'
    phases = [dict(phase=phase, outcome='passed') for phase in ['setup', 'call', 'teardown']]
    ledger = {case: dict(kind='native-helper', check='graph_lazy_sql_plan', entry='_class_has_assignment')}
    record = dict(test=case, check='graph_lazy_sql_plan', entry='_class_has_assignment', files=[], parameters={}, expected=True)
    oracle = dict(cases={case: phases}, records=[record])
    call = dict(test=case, check=record['check'], entry=record['entry'], contract='batch-ten-helper-ast', findings=1, metrics={})
    report = dict(oracle=False, mode='snapshot', exitCode=0, cases=oracle['cases'], helperRecords=[record], helperCalls=[call],
                  helperContractsSha256='contracts', engineSha256='engine', semanticFallbacks=[], assertionTranslations=[])
    return ledger, oracle, copy.deepcopy(report)


def test_original_python_passes_cannot_replace_native_helper_execution():
    ledger, oracle, report = example()
    report['helperCalls'] = []
    with pytest.raises(ValueError, match='native helper call'):
        verify_helpers(ledger, oracle, [report], 'snapshot', 'contracts')


def test_every_declared_rule_helper_case_requires_an_observation():
    ledger, oracle, report = example()
    report['helperRecords'] = []; report['helperCalls'] = []
    with pytest.raises(ValueError, match='Missing native helper'):
        verify_helpers(ledger, oracle, [report], 'snapshot', 'contracts')


def test_false_booleans_require_a_native_row_too():
    ledger, oracle, report = example()
    oracle['records'][0]['expected'] = report['helperRecords'][0]['expected'] = False
    report['helperCalls'][0]['findings'] = 0
    with pytest.raises(ValueError, match='native helper call'):
        verify_helpers(ledger, oracle, [report], 'snapshot', 'contracts')


def test_helper_result_inputs_and_programs_are_bound():
    ledger, oracle, report = example()
    with pytest.raises(ValueError, match='Changed helper programs'):
        verify_helpers(ledger, oracle, [report], 'snapshot', 'changed')
    report['helperRecords'][0]['expected'] = False
    with pytest.raises(ValueError, match='Complete helper parity'):
        verify_helpers(ledger, oracle, [report], 'snapshot', 'contracts')


def test_original_runs_and_the_wrong_api_cannot_certify_native_helpers():
    ledger, oracle, report = example()
    report['oracle'] = True
    with pytest.raises(ValueError, match='Original execution'):
        verify_helpers(ledger, oracle, [report], 'snapshot', 'contracts')
    report['oracle'] = False
    with pytest.raises(ValueError, match='Wrong helper API'):
        verify_helpers(ledger, oracle, [report], 'workspace', 'contracts')
