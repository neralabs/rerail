# SPDX-License-Identifier: Apache-2.0
"""Require every portable assertion, native vocabulary case, and removal control."""
import ast
from collections import Counter
import hashlib
from .transfer import classify


def verify_transfer(ledger, oracle, reports):
    wanted = {name for name, value in ledger.items() if value['kind'] == 'host'}
    if set(oracle['cases']) != wanted or set(oracle['originalCaseOutcomes']) != wanted:
        raise ValueError('Missing or extra audited host cases')
    if not oracle.get('sourceFilesChecked'):
        raise ValueError('Original host sources were not pinned')
    counts = Counter()
    expected = {'preserves native command paths across working directories', 'rejects disabled native transport of the original CLI positives',
                'transfers command help without loading a repository'}
    transferred = []
    python_only = {}
    for name, case in oracle['cases'].items():
        if hashlib.sha256(case['source'].encode()).hexdigest() != case['sha256']:
            raise ValueError('Changed original assertion source')
        phases = oracle['originalCaseOutcomes'][name]
        if len(phases) != 3 or any(phase['outcome'] != 'passed' for phase in phases):
            raise ValueError('Original host assertion did not pass')
        node = ast.parse(case['source']).body[0]
        assertions = []
        for item in ast.walk(node):
            expressions = [item.test] if isinstance(item, ast.Assert) else [c.context_expr for c in item.items if 'raises(' in ast.unparse(c.context_expr)] if isinstance(item, ast.With) else []
            for expression in expressions:
                contract, reason = classify(node.name, expression)
                assertions.append(dict(source=ast.unparse(expression), contract=contract, reason=reason))
                counts[contract or 'pythonOnly'] += 1
        if assertions != case['assertions'] or not assertions:
            raise ValueError('Missing or changed assertion decision')
        if any(assertion['contract'] for assertion in assertions):
            transferred.append(name)
            expected.add('transfers upstream assertions: ' + name)
        retained = [assertion for assertion in assertions if not assertion['contract']]
        if retained:
            python_only[name] = retained
        if any(assertion['contract'] == 'command' for assertion in assertions) and 'command' not in case:
            raise ValueError('Missing original command observation')
    groups = oracle['bannedGroups']
    symbols = {symbol for group in groups for symbol in group['symbols']}
    if len(groups) != 2 or len(symbols) != sum(len(group['symbols']) for group in groups) or not all(group['symbols'] for group in groups):
        raise ValueError('Invalid original banned vocabularies')
    found = set()
    for record in oracle['detectorCases']:
        key = record['symbol'], record['variant']
        if key in found:
            raise ValueError('Duplicate native vocabulary observation')
        found.add(key)
        if len(record['expected']) != (1 if record['variant'] == 'positive' else 0):
            raise ValueError('Missing vocabulary positive or clean control')
        expected.add('executes transferred vocabulary ' + record['test'] + ' through both native APIs')
    if found != {(symbol, variant) for symbol in symbols for variant in ['positive', 'lookalike', 'other-family']}:
        raise ValueError('Missing native vocabulary observation')
    expected.update('rejects removal of transferred symbol ' + symbol for symbol in symbols)
    seen = set()
    for report in reports:
        if report['numFailedTests'] or report.get('numRuntimeErrorTestSuites', 0):
            raise ValueError('Transferred native assertions failed')
        for suite in report['testResults']:
            for assertion in suite['assertionResults']:
                if assertion['status'] == 'passed':
                    if assertion['fullName'] in seen:
                        raise ValueError('Duplicate transferred assertion execution')
                    seen.add(assertion['fullName'])
    if seen != expected:
        raise ValueError('Missing or extra transferred native assertions: ' + str(expected ^ seen))
    return dict(sourceCases=len(wanted), transferredCaseIds=sorted(transferred), transferredCases=len(transferred),
                assertionSites=dict(counts), pythonOnlyAssertions=python_only,
                bannedSymbols=len(symbols), detectorObservations=len(found), detectorApiComparisons=2 * len(found),
                symbolRemovalControls=len(symbols), nativeTests=len(seen), missing=[])
