# SPDX-License-Identifier: Apache-2.0
"""Verify native helper executions separately from detector and host coverage."""
from collections import Counter, defaultdict

def verify_replay(oracle,reports):
    entries={record['entry'] for record in oracle['records']}
    expected={name for entry in entries for name in [f'matches original helper {entry} through both native APIs',f'rejects disabled native helper {entry}']}
    seen=set()
    for report in reports:
        if report['numFailedTests'] or report.get('numRuntimeErrorTestSuites',0):
            raise ValueError('Native helper replay failed')
        for suite in report['testResults']:
            for case in suite['assertionResults']:
                if case['status']=='passed':
                    if case['fullName'] in seen:raise ValueError('Duplicate helper replay assertion')
                    seen.add(case['fullName'])
    if seen!=expected:raise ValueError('Missing or extra helper replay assertions')
    return dict(observations=len(oracle['records']),supplemental=len(oracle['supplemental']),apiComparisons=2*(len(oracle['records'])+len(oracle['supplemental'])),disabledHelpers=len(entries),assertions=len(seen))


def contract(entry):
    return 'batch-ten-helper-' + {'_parse_source': 'parse', '_is_data_path': 'path',
        '_approval_entry_errors': 'approvals', '_tree_uses_matching_contracts': 'matching'}.get(entry, 'ast')


def rows(record):
    value = record['expected']
    if isinstance(value, bool) or record['entry'] == '_parse_source':
        return 1
    if isinstance(value, list):
        return len(value)
    return 0 if value is None else 1


def verify_helpers(ledger, oracle, reports, mode, contracts_hash):
    wanted = {case for case, value in ledger.items() if value['kind'] == 'native-helper'}
    if wanted != set(oracle['cases']) or wanted != {record['test'] for record in oracle['records']}:
        raise ValueError('Helper oracle does not cover the declared unit assertions')
    observed = set(); records = []; calls = []; translations = []; engine = None
    for report in reports:
        if report.get('oracle') is not False:
            raise ValueError('Original execution cannot certify native helper assertions')
        if report['mode'] != mode:
            raise ValueError('Wrong helper API mode')
        if report['exitCode'] != 0 or report['semanticFallbacks']:
            raise ValueError('Native helper assertions failed or used original semantics')
        selected = wanted & set(report['cases'])
        helper_records = report.get('helperRecords', [])
        helper_calls = report.get('helperCalls', [])
        if (selected or helper_records or helper_calls) and report.get('helperContractsSha256') != contracts_hash:
            raise ValueError('Changed helper programs')
        if engine is not None and engine != report['engineSha256']:
            raise ValueError('Mixed native helper builds')
        engine = report['engineSha256']
        if observed & selected:
            raise ValueError('Duplicate helper assertion coverage')
        observed.update(selected)
        for case in selected:
            phases = report['cases'][case]
            if len(phases) != 3 or any(phase['outcome'] != 'passed' for phase in phases):
                raise ValueError('Incomplete native helper assertion: ' + case)
        records.extend(helper_records); calls.extend(helper_calls)
        translations.extend(report.get('assertionTranslations', []))
    if observed != wanted or {record['test'] for record in records} != wanted:
        raise ValueError('Missing native helper observations')
    key = lambda value: (value['test'], value['check'], value['entry'])
    def grouped(values):
        result = defaultdict(list)
        for value in values:
            result[key(value)].append(value)
        return dict(result)
    if grouped(oracle['records']) != grouped(records):
        raise ValueError('Complete helper parity differs from the unchanged original assertions')
    by_key = grouped(calls)
    for record in records:
        matching = by_key.get(key(record), [])
        if not matching:
            raise ValueError('Missing native helper call trace')
        call = matching.pop(0)
        if call['contract'] != contract(record['entry']) or call['findings'] != rows(record):
            raise ValueError('Inconsistent native helper call trace')
    if any(by_key.values()):
        raise ValueError('Unrecorded native helper calls')
    instrumented = {case for case, value in ledger.items() if value.get('translation')}
    if {translation['test'] for translation in translations} != instrumented or len(translations) != len(instrumented):
        raise ValueError('Missing or extra native instrumentation translation')
    for translation in translations:
        if translation['nativeCounters'] != ['pythonAstModuleWalks', 'pythonAstParses'] or not all(translation.get(field) for field in ['originalHash', 'translatedHash']):
            raise ValueError('Unbound native instrumentation assertion')
        matching = [call for call in calls if call['test'] == translation['test']]
        if len(matching) != 1:
            raise ValueError('Missing native module-walk measurement')
        metrics = matching[0]['metrics'].get('workspaceProfile', matching[0]['metrics'])
        if any(metrics.get(field) != 1 for field in translation['nativeCounters']):
            raise ValueError('Native helper did not parse and walk the module exactly once')
    return dict(nativeCaseIds=sorted(wanted), cases=len(wanted), completeResults=len(records), calls=len(calls),
                perEntry=dict(Counter(record['entry'] for record in records)), instrumentationTranslations=translations,
                engineSha256=engine, contractsSha256=contracts_hash)
