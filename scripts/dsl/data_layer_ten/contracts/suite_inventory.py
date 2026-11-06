# SPDX-License-Identifier: Apache-2.0
"""Inventory original tests independently of recorded observations and port selection."""
import argparse
import ast
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from data_layer_ten.common import CHECKS, PIN

SHARED = {'conditional_architecture_facts': 'conditional_architecture'}


def references(text):
    tree = ast.parse(text)
    modules = set()
    strings = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            modules.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            modules.add(node.module or '')
            modules.update((node.module + '.' if node.module else '') + alias.name for alias in node.names)
        elif isinstance(node, ast.Constant) and isinstance(node.value, str):
            strings.append(node.value)
    detectors = {check for check in CHECKS if modules & {'scripts.lint_' + check, 'lint_' + check}}
    # Literal module dependencies also cover import_module, __import__, and parameterized module lists.
    detectors.update(check for check in CHECKS if any(value in {'scripts.lint_' + check, 'lint_' + check} for value in strings))
    named = {check for check in CHECKS if check in strings}
    integration = named if 'scripts.lint_all' in modules else set()
    detectors.update(named - integration)
    detectors.update(owner for module, owner in SHARED.items() if modules & {module, 'scripts.' + module})
    adjacent = {check for check in CHECKS if any(re.search(r'(?<!\w)(?:scripts/)?lint_' + check + r'\.py\b', value) for value in strings)}
    return dict(detectors=sorted(detectors), workflowReferences=sorted(adjacent - detectors), integrationReferences=sorted(integration - detectors))


def inventory(source, output):
    listing = subprocess.check_output(['git', '-C', str(source), 'ls-tree', '-rz', PIN]).split(b'\0')
    blobs = {}
    for row in listing:
        if row:
            metadata, name = row.split(b'\t', 1); blobs[name.decode()] = metadata.split()[2].decode()
    files = {}; scanned = 0; unparsed = []
    for name in sorted(blobs):
        path = Path(name)
        if not path.name.startswith('test_') or path.suffix != '.py':
            continue
        data = (source / name).read_bytes(); scanned += 1
        if hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() != blobs[name]:
            raise ValueError('Changed original test module: ' + name)
        try:
            owners = references(data.decode('utf-8'))
        except SyntaxError as error:
            unparsed.append(dict(path=name, error=str(error))); continue
        if any(owners.values()):
            files[name] = dict(**owners, blob=blobs[name], sha256=hashlib.sha256(data).hexdigest())
    if unparsed:
        raise ValueError('Cannot certify discovery with unparsed test modules: ' + str(unparsed))
    cases = {}; adjacent = {}; integration = {}
    with tempfile.TemporaryDirectory(prefix='rerail-original-test-inventory-') as directory:
        def collect(item):
            index, (name, module) = item
            path = Path(directory) / (str(index) + '.json')
            result = subprocess.run([sys.executable, str(Path(__file__).with_name('inventory_collect.py')), str(source), name, str(path)],
                capture_output=True, text=True, timeout=110, env={**os.environ, 'PYTEST_ADDOPTS': ''})
            if result.returncode:
                raise ValueError('Original collection failed: ' + name + '\n' + result.stdout + result.stderr)
            return module, json.loads(path.read_text())
        # Isolated pytest processes preserve module ownership. Two workers bound startup cost.
        with ThreadPoolExecutor(max_workers=2) as workers:
            collected_modules = list(workers.map(collect, enumerate(files.items())))
        for module, collected in collected_modules:
            target = cases if module['detectors'] else integration if module['integrationReferences'] else adjacent
            for identity, case in collected['cases'].items():
                if identity in target:
                    raise ValueError('Duplicate original case: ' + identity)
                target[identity] = case
            module['cases'] = len(collected['cases'])
    result = dict(format='rerail.data-layer-original-test-inventory/v1', sourcePin=PIN, scannedTestFiles=scanned,
                  unparsedTestFiles=unparsed, files=files, cases=cases, adjacentWorkflowCases=adjacent,suiteIntegrationCases=integration,
                  missingDedicatedSuites=[check for check in CHECKS if f'scripts/tests/test_lint_{check}.py' not in blobs],
                  note='Whole original modules are collected in isolated processes without port adapters or test filters. Detector and shared-helper imports select original suites. Workflow script references are inventoried separately. Assert sites are static source inventory, not a claim that every runtime branch executed.')
    output.write_text(json.dumps(result, indent=2) + '\n')
    return result


def verify_execution(inventory, reports, mode, ledger=None):
    wanted = set(inventory['cases']); seen = set(); records = []; helpers = []
    for report in reports:
        if report.get('oracle') is not False or report['mode'] != mode or report['exitCode'] or report['semanticFallbacks']:
            raise ValueError('Original suite did not execute successfully through native ' + mode)
        selected = wanted & set(report['cases'])
        if seen & selected:
            raise ValueError('Duplicate original execution: ' + str(seen & selected))
        seen.update(selected)
        for name in selected:
            phases = report['cases'][name]
            if sorted(phase['phase'] for phase in phases) != ['call', 'setup', 'teardown'] or any(phase['outcome'] != 'passed' for phase in phases):
                raise ValueError('Incomplete original runtime case: ' + name)
        records.extend(record for record in report['records'] if record['test'] in selected)
        helpers.extend(record for record in report['helperRecords'] if record['test'] in selected)
    if seen != wanted:
        raise ValueError('Missing original runtime cases: ' + str(wanted - seen))
    detector_cases = {record['test'] for record in records}; helper_cases = {record['test'] for record in helpers}
    hosts = {name for name, case in (ledger or {}).items() if case['kind'] == 'host'} & wanted
    if detector_cases | helper_cases | hosts != wanted:
        raise ValueError('Unclassified original runtime cases: ' + str(wanted - detector_cases - helper_cases - hosts))
    return dict(cases=len(wanted), detectorCases=len(detector_cases), detectorObservations=len(records),
                positiveDetectorObservations=sum(bool(record['expected']) for record in records),
                helperCases=len(helper_cases), helperObservations=len(helpers), hostCases=len(hosts),
                missing=[], skipped=[], failed=[], perFile=dict(Counter(case['file'] for case in inventory['cases'].values())))


def verify_integration(inventory, original, native):
    wanted = set(inventory['suiteIntegrationCases']) | set(inventory['adjacentWorkflowCases'])
    if original.get('oracle') is not True or original['sourcePin'] != inventory['sourcePin'] or original['exitCode'] or original['errors'] or original['deselected']:
        raise ValueError('Original integration execution failed')
    if set(original['cases']) != wanted or set(original['collected']) != wanted:
        raise ValueError('Missing original integration cases')
    if original['collected'] != {**inventory['suiteIntegrationCases'], **inventory['adjacentWorkflowCases']}:
        raise ValueError('Changed original integration assertion bodies')
    for name, phases in original['cases'].items():
        if sorted(phase['phase'] for phase in phases) != ['call', 'setup', 'teardown'] or any(phase['outcome'] != 'passed' for phase in phases):
            raise ValueError('Incomplete original integration case: ' + name)
    prefix = 'scripts/tests/test_lint_all.py::test_serverplan_linters_are_ci_blockers['
    checks = sorted(check for check in CHECKS if prefix + check + ']' in wanted)
    expected = {'preserves original registry CI blocking with the shipped ' + check + ' rule' for check in checks}
    if native.get('oracle') is True:
        raise ValueError('Original execution cannot replace native registry counterparts')
    if native['numFailedTests'] or native.get('numRuntimeErrorTestSuites', 0):
        raise ValueError('Native registry counterparts failed')
    seen = [case['fullName'] for suite in native['testResults'] for case in suite['assertionResults'] if case['status'] == 'passed']
    if len(seen) != len(set(seen)) or set(seen) != expected:
        raise ValueError('Missing native registry counterparts')
    return dict(originalRegistryCases=len(inventory['suiteIntegrationCases']), originalGateCases=len(inventory['adjacentWorkflowCases']),
                nativeRegistryCounterparts=len(checks), positiveAndCleanCommandComparisons=2 * len(checks), missing=[], skipped=[], failed=[])


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True); parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(); inventory(args.source.resolve(), args.output)
