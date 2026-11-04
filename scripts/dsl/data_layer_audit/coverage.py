#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Audit complete scanner-call replay and report per-case coverage boundaries."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path


def identity(row):
    return row['test'], row['check'], row['call']


def audit_calls(events, rows):
    expected = {}
    for event in events:
        if event.get('retained_reason') or not event['upstream_passed']:
            continue
        key = identity(event)
        if key in expected:
            raise ValueError(f'Duplicate original call: {key}')
        expected[key] = event
    actual = {}
    for row in rows:
        key = identity(row)
        if key in actual:
            raise ValueError(f'Duplicate replay call: {key}')
        actual[key] = row
    for key, event in expected.items():
        row = actual.pop(key, None)
        if row is None:
            raise ValueError(f'Missing original call: {key}')
        if (row['execution'] != 'native' or row['expected'] != len(event['findings'])
                or row['actual'] != len(event['findings'])
                or row['positive'] != bool(event['findings'])
                or any(row.get(field) for field in ('falsePositives', 'falseNegatives',
                                                  'engineFailures', 'statelessFailures', 'executionPathDifferences'))):
            raise ValueError(f'Original call differs: {key}')
    if actual:
        raise ValueError(f'Unexpected replay calls: {list(actual)}')
    return dict(Counter(key[0] for key in expected))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('recording', 'probes', 'parity', 'discovery', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--upstream', type=Path, action='append', required=True)
    args = parser.parse_args()
    read = lambda path: json.loads(path.read_text())
    recording, probes, parity, discovery = map(read, (args.recording, args.probes, args.parity, args.discovery))
    calls = audit_calls(recording['events'] + probes['events'], parity['rows'])
    helper_calls = Counter(event['test'] for event in recording['helper_events'] if 'functions' in event)
    if (helper_calls != Counter(discovery['tests']) or discovery['failures'] or discovery['differences']):
        raise ValueError('Discovery observations were lost or differ')
    upstream = []
    for path in args.upstream:
        original = read(path)
        if original['source_mismatches'] or original['pytest_exit_code'] or original['commit'] != recording['commit']:
            raise ValueError(f'Unverified upstream tests: {path}')
        outcomes = []
        for test, phases in original['test_outcomes'].items():
            if any(phase['outcome'] == 'failed' for phase in phases):
                raise ValueError(f'Failed upstream case: {test}')
            outcomes.append(next((phase['outcome'] for phase in phases if phase['phase'] == 'call'), 'skipped'))
        if len(outcomes) != original['collected_tests']:
            raise ValueError(f'Missing upstream case outcomes: {path}')
        upstream.append({'recording':str(path), 'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
                         'collected':len(outcomes), 'passed':outcomes.count('passed'), 'skipped':outcomes.count('skipped')})
    if (recording['pytest_exit_code'] or recording['source_mismatches'] or probes['source_mismatches']
            or probes['commit'] != recording['commit'] or parity['commit'] != recording['commit']):
        raise ValueError('Scanner observations lack verified source provenance')
    cases = []
    for test, phases in recording['test_outcomes'].items():
        events = [event for event in recording['events'] if event['test'] == test]
        cases.append({'test':test, 'phases':phases, 'nativeScannerCalls':calls.get(test, 0),
                      'nativePositiveCalls':sum(bool(event.get('findings')) for event in events if not event.get('retained_reason')),
                      'nativeDiscoveryCalls':helper_calls[test],
                      'originalFactSourceInputs':sum(probe['test'] == test for probe in recording['source_probes']),
                      'retainedCalls':[event['retained_reason'] for event in events if event.get('retained_reason')],
                      'upstreamAssertionsOnly':not (calls.get(test, 0) or helper_calls[test])})
    captured = {event['test'] for event in recording['events']}
    if set(recording['uncaptured_tests']) != set(recording['test_outcomes']) - captured:
        raise ValueError('Uncaptured test inventory is stale')
    report = {'format':'rerail.data-layer-case-coverage/v1', 'commit':recording['commit'],
              'nativeSha256':parity['nativeSha256'], 'upstream':upstream,
              'inputs':{str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in (args.recording,args.probes)},
              'upstreamCollected':sum(row['collected'] for row in upstream),
              'upstreamPassed':sum(row['passed'] for row in upstream),
              'upstreamSkipped':sum(row['skipped'] for row in upstream),
              'batchEightNativeCalls':parity['nativeCalls'], 'batchEightPositiveCalls':parity['positiveCalls'],
              'batchEightCleanCalls':parity['cleanCalls'],
              'discovery':discovery, 'cases':cases,
              'qualifications':[
                  'Native scanner counts prove every eligible recorded call was replayed, including every positive.',
                  'Scanner parity compares finding multiplicity, path, line and code, plus recorded semantic fields.',
                  'Cached discovery compares every DecoratedFunction field through snapshot and workspace APIs.',
                  'Original helper, CLI, signing, I/O and Python structure assertions are separate from native scanner observations.',
                  'Original fact-source inputs do not certify the helper assertions that inspect them.',
                  'Diagnostic prose is not compared unless it is the recorded diagnostic identity.',
                  'No full-repository cold execution was performed for this audit.']}
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key:value for key,value in report.items() if key not in ('upstream','cases','discovery','qualifications')}))


if __name__ == '__main__':
    main()
