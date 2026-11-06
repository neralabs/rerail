# SPDX-License-Identifier: Apache-2.0
"""The independent inventory must expose tests missing from both stored reports."""
import copy
import pytest

from data_layer_ten.contracts.suite_inventory import references, verify_execution


@pytest.mark.parametrize('text', [
    'from scripts import lint_hnsw_architecture as linter\n',
    'from scripts.lint_hnsw_architecture import find_hnsw_architecture_findings\n',
    'import scripts.lint_hnsw_architecture as linter\n',
    'import lint_hnsw_architecture as linter\n',
    'linter = importlib.import_module("scripts.lint_hnsw_architecture")\n',
    'linter = __import__("scripts.lint_hnsw_architecture")\n',
    'MODULES = ["scripts.lint_hnsw_architecture"]\n',
    'class TestOwner:\n    def test_owner(self):\n        from scripts import lint_hnsw_architecture\n',
])
def test_direct_dynamic_and_class_dependencies_select_the_entire_module(text):
    assert references(text)['detectors'] == ['hnsw_architecture']


def test_shared_conditional_suite_is_discovered_without_a_filename_exception():
    assert references('from scripts.conditional_architecture_facts import conditional_architecture_findings\n')['detectors'] == ['conditional_architecture']


def test_workflow_script_references_are_kept_separate_from_detector_calls():
    assert references('assert "scripts/lint_matching_architecture.py --check" in command\n') == {
        'detectors': [], 'workflowReferences': ['matching_architecture']}


def example():
    case = 'scripts/tests/test_rule.py::TestRule::test_positive[cell]'
    inventory = {'cases': {case: {'file': 'scripts/tests/test_rule.py'}}}
    phases = [dict(phase=name, outcome='passed') for name in ['setup', 'call', 'teardown']]
    report = dict(oracle=False, mode='snapshot', exitCode=0, semanticFallbacks=[], cases={case: phases},
                  records=[dict(test=case, expected=[{'code': 'HNSW001'}])], helperRecords=[])
    return inventory, report


def test_a_case_missing_from_both_the_recording_and_execution_still_fails():
    inventory, report = example()
    report['cases'] = {}; report['records'] = []
    with pytest.raises(ValueError, match='Missing original'):
        verify_execution(inventory, [report], 'snapshot')


def test_duplicated_shards_do_not_replace_missing_original_cases():
    inventory, report = example()
    with pytest.raises(ValueError, match='Duplicate original'):
        verify_execution(inventory, [report, copy.deepcopy(report)], 'snapshot')


@pytest.mark.parametrize('status', ['skipped', 'failed', 'xfailed'])
def test_all_original_runtime_phases_must_pass(status):
    inventory, report = example()
    report['cases'][next(iter(inventory['cases']))][1]['outcome'] = status
    with pytest.raises(ValueError, match='Incomplete original'):
        verify_execution(inventory, [report], 'snapshot')


def test_an_unclassified_original_case_cannot_silently_remain_on_python():
    inventory, report = example()
    report['records'] = []
    with pytest.raises(ValueError, match='Unclassified original'):
        verify_execution(inventory, [report], 'snapshot')
