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


def test_python_dto_construction_cannot_mask_a_native_parse_error():
    from data_layer_ten.assertions.adapter import decode
    rows = [{'variables': {'row': {'helper_value': 'null'}}}]
    with pytest.raises(AssertionError, match='Native parser accepted invalid source'):
        decode('_parse_source', rows, [{'path': 'helper.py', 'source': 'def broken(:\n'}], {})


def test_unrelated_detector_reports_cannot_stand_in_for_helper_reports():
    ledger, oracle, report = example()
    core = dict(oracle=False,mode='snapshot',exitCode=0,cases={'detector-case':[]},engineSha256='engine',semanticFallbacks=[])
    summary=verify_helpers(ledger,oracle,[report,core],'snapshot','contracts')
    assert summary['cases']==1 and summary['calls']==1
    with pytest.raises(ValueError,match='Missing native helper'):
        verify_helpers(ledger,oracle,[core],'snapshot','contracts')


def test_field_name_sets_transfer_but_python_hashability_does_not():
    import ast
    from data_layer_ten.assertions.transfer import classify
    assert classify('test_violation_has_four_fields', ast.parse("names == {'code', 'file_path', 'line_number', 'message'}", mode='eval').body)[0] == 'values'
    assert classify('test_violation_is_hashable', ast.parse('v in s', mode='eval').body)[0] is None


def transfer_example():
    import json
    from data_layer_rule_audit import fixtures_path
    root = fixtures_path('host', 'data-layer-port')
    ledger = json.loads((root / 'batch-10-assertion-ledger.json').read_text())['cases']
    oracle = json.loads((root / 'batch-10-transferable-assertions.json').read_text())
    names = ['transfers upstream assertions: ' + name for name, case in oracle['cases'].items() if any(a['contract'] for a in case['assertions'])]
    names += ['executes transferred vocabulary ' + case['test'] + ' through both native APIs' for case in oracle['detectorCases']]
    names += ['rejects removal of transferred symbol ' + symbol for group in oracle['bannedGroups'] for symbol in group['symbols']]
    names += ['preserves native command paths across working directories', 'rejects disabled native transport of the original CLI positives', 'transfers command help without loading a repository']
    report = dict(numFailedTests=0, testResults=[dict(assertionResults=[dict(fullName=name, status='passed') for name in names])])
    return ledger, oracle, report


@pytest.mark.parametrize('defect', ['missing-test', 'duplicate-test', 'untranslated-assertion', 'missing-control', 'changed-source'])
def test_transfer_proof_rejects_incomplete_or_changed_coverage(defect):
    from data_layer_ten.assertions.transferred import verify_transfer
    ledger, oracle, report = transfer_example()
    assert verify_transfer(ledger, oracle, [report])['missing'] == []
    assertions = report['testResults'][0]['assertionResults']
    if defect == 'missing-test':
        assertions.pop(0)
    elif defect == 'duplicate-test':
        assertions.append(assertions[0])
    elif defect == 'untranslated-assertion':
        case = next(case for case in oracle['cases'].values() if any(a['contract'] for a in case['assertions']))
        next(a for a in case['assertions'] if a['contract'])['contract'] = None
    elif defect == 'missing-control':
        oracle['detectorCases'].pop()
    elif defect == 'changed-source':
        next(iter(oracle['cases'].values()))['source'] += '\n# changed\n'
    with pytest.raises(ValueError):
        verify_transfer(ledger, oracle, [report])
