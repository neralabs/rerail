# SPDX-License-Identifier: Apache-2.0
"""The independent inventory must expose tests missing from both stored reports."""
import copy
import pytest

from data_layer_ten.contracts.suite_inventory import references, verify_execution, verify_integration


@pytest.mark.parametrize('text', [
    'from scripts import lint_hnsw_architecture as linter\n',
    'from scripts.lint_hnsw_architecture import find_hnsw_architecture_findings\n',
    'import scripts.lint_hnsw_architecture as linter\n',
    'import lint_hnsw_architecture as linter\n',
    'linter = importlib.import_module("scripts.lint_hnsw_architecture")\n',
    'linter = __import__("scripts.lint_hnsw_architecture")\n',
    'MODULES = ["scripts.lint_hnsw_architecture"]\n',
    'linter = plugin("hnsw_architecture")\n',
    'class TestOwner:\n    def test_owner(self):\n        from scripts import lint_hnsw_architecture\n',
])
def test_direct_dynamic_and_class_dependencies_select_the_entire_module(text):
    assert references(text)['detectors'] == ['hnsw_architecture']


def test_shared_conditional_suite_is_discovered_without_a_filename_exception():
    assert references('from scripts.conditional_architecture_facts import conditional_architecture_findings\n')['detectors'] == ['conditional_architecture']


def test_workflow_script_references_are_kept_separate_from_detector_calls():
    assert references('assert "scripts/lint_matching_architecture.py --check" in command\n') == {
        'detectors': [], 'workflowReferences': ['matching_architecture'], 'integrationReferences': []}


def test_registry_parameters_remain_visible_as_integration_contracts():
    text = 'from scripts.lint_all import DL_SUITE\nNAMES = ["graph_lazy_sql_plan", "conditional_architecture"]\n'
    assert references(text) == {'detectors': [], 'workflowReferences': [],
                                'integrationReferences': ['conditional_architecture', 'graph_lazy_sql_plan']}


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


def test_independent_collection_retains_parameterized_class_methods(tmp_path):
    import json
    import os
    from pathlib import Path
    import subprocess
    import sys
    (tmp_path / 'pyproject.toml').write_text('[tool.pytest.ini_options]\n')
    (tmp_path / 'test_class.py').write_text('import pytest\nclass TestOwner:\n    @pytest.mark.parametrize("value", [1, 2])\n    def test_value(self, value):\n        assert value > 0\n    def test_plain(self):\n        assert True\n')
    output = tmp_path / 'collected.json'
    worker = Path(__file__).with_name('inventory_collect.py')
    result = subprocess.run([sys.executable, str(worker), str(tmp_path), 'test_class.py', str(output)],
                            capture_output=True, text=True, env={**os.environ, 'PYTEST_DISABLE_PLUGIN_AUTOLOAD': '1'}, timeout=20)
    assert result.returncode == 0, result.stdout + result.stderr
    collected = json.loads(output.read_text())
    assert set(collected['cases']) == {'test_class.py::TestOwner::test_value[1]', 'test_class.py::TestOwner::test_value[2]', 'test_class.py::TestOwner::test_plain'}
    assert all(case['assertSites'] for case in collected['cases'].values())


@pytest.mark.parametrize('defect', ['missing-original', 'skipped-original', 'changed-original', 'missing-native', 'duplicate-native', 'failed-native', 'original-as-native'])
def test_registry_proof_requires_both_original_execution_and_native_counterparts(defect):
    import json
    from pathlib import Path
    from data_layer_ten.common import CHECKS
    inventory = json.loads(Path('fixtures/host/data-layer-port/batch-10-original-test-inventory.json').read_text())
    names = set(inventory['suiteIntegrationCases']) | set(inventory['adjacentWorkflowCases'])
    phases = lambda: [dict(phase=name, outcome='passed') for name in ['setup', 'call', 'teardown']]
    original = dict(oracle=True, sourcePin=inventory['sourcePin'], exitCode=0, errors=[], deselected=[],
                    cases={name: phases() for name in names}, collected=copy.deepcopy({**inventory['suiteIntegrationCases'], **inventory['adjacentWorkflowCases']}))
    prefix = 'scripts/tests/test_lint_all.py::test_serverplan_linters_are_ci_blockers['
    native_names = ['preserves original registry CI blocking with the shipped ' + check + ' rule'
                    for check in CHECKS if prefix + check + ']' in names]
    rows = [dict(fullName=name, status='passed') for name in native_names]
    native = dict(numFailedTests=0, testResults=[dict(assertionResults=rows)])
    assert verify_integration(inventory, original, native)['nativeRegistryCounterparts'] == 3
    if defect == 'missing-original': original['cases'].pop(next(iter(names)))
    elif defect == 'skipped-original': original['cases'][next(iter(names))][1]['outcome'] = 'skipped'
    elif defect == 'changed-original': original['collected'][next(iter(names))]['definitionSha256'] = 'changed'
    elif defect == 'missing-native': rows.pop()
    elif defect == 'duplicate-native': rows.append(rows[0])
    elif defect == 'failed-native': rows[0]['status'] = 'failed'
    else: native = original
    with pytest.raises(ValueError): verify_integration(inventory, original, native)
