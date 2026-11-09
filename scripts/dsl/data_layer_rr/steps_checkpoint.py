"""Audit a partial step-contract checkpoint against every selected original observation."""
import hashlib
import json

from migration import PIN, compare


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=True,
                                    separators=(',', ':')).encode()).hexdigest()


def assemble(source, reports):
    if source['sourcePin'] != PIN or source['pytest_exit_code'] != 0:
        raise ValueError('A successful pinned source recording is required')
    calls = {}
    for call in source['calls']:
        calls.setdefault(call['test'], []).append(call)
    seen, parts, selected, observations, native_calls = set(), None, [], [], []
    entries = engine = programs = harness = None
    for report in reports:
        if report['sourcePin'] != PIN or report['exitCode'] not in (0, 5):
            raise ValueError('A native shard failed or used another source')
        if report['semanticFallbacks'] or report['hostProtocolCalls'] or report['missingResults']:
            raise ValueError('A native shard omitted results or invoked source semantics')
        compare(source['collected'], report['collected'])
        identity = report['part']
        if identity in seen or not 0 <= identity < report['parts']:
            raise ValueError('Duplicate or invalid native shard')
        seen.add(identity)
        if parts is None:
            parts, entries, engine, harness = report['parts'], report['selectedEntries'], report['engineSha256'], report['harnessFiles']
        compare((parts, entries, engine, harness),
                (report['parts'], report['selectedEntries'], report['engineSha256'], report['harnessFiles']))
        if not entries:
            raise ValueError('A partial checkpoint must state its selected entry points')
        allowed = set(entries.split(','))
        expected_cases = [test for test in source['collected'][identity::parts]
                          if calls.get(test) and all(c['entry'] in allowed for c in calls[test])]
        if report['exitCode'] == 5 and expected_cases:
            raise ValueError('Pytest collected no tests for a nonempty assertion shard')
        compare(expected_cases, report['selectedCases'])
        if not report['nativeCalls'] and any(c['entry'] != 'Violation.__init__'
                                             for test in expected_cases for c in calls[test]):
            raise ValueError('A semantic checkpoint shard has no native execution')
        if set(report['cases']) != set(expected_cases):
            raise ValueError('A selected original assertion is absent')
        for test in expected_cases:
            phases = report['cases'][test]
            if [p['phase'] for p in phases] != ['setup', 'call', 'teardown'] or any(p['outcome'] != 'passed' for p in phases):
                raise ValueError('A selected original assertion did not pass')
            actual = [c for c in report['calls'] if c['test'] == test]
            compare(calls[test], actual)
            for ordinal, call in enumerate(actual):
                thin = dict(call)
                thin['argumentSha256'] = digest(thin.pop('arguments'))
                observations.append(dict(test=test, ordinal=ordinal, actual=thin))
        if any(c['test'] not in expected_cases for c in report['calls']):
            raise ValueError('An observation belongs to an unselected original assertion')
        for call in report['nativeCalls']:
            if call['status'] != 'passed':
                raise ValueError('A native execution failed')
            if programs is None:
                programs = call['programInputs']
            compare(programs, call['programInputs'])
            native_calls.append(call)
        selected.extend(expected_cases)
    if parts is None or seen != set(range(parts)):
        raise ValueError('The native checkpoint omits a shard')
    selected = [t for t in source['collected'] if t in set(selected)]
    return dict(format='rerail.serverplan-steps-partial-checkpoint/v1', sourcePin=PIN,
                fullMigrationAcceptance=False, sourceAssertions=len(source['collected']),
                selectedEntries=entries.split(','), selectedAssertions=len(selected), selectedCases=selected,
                untestedCases=[t for t in source['collected'] if t not in selected],
                observations=observations, nativeCalls=native_calls, programInputs=programs,
                engineSha256=engine, harnessFiles=harness)
