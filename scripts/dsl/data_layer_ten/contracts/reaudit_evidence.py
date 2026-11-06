# SPDX-License-Identifier: Apache-2.0
"""Keep fresh semantic comparisons separate from historical repository timings."""
import hashlib
import json
from pathlib import Path
from collections import Counter

from data_layer_ten.common import CHECKS, PIN


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify(oracle, reports):
    if oracle['sourcePin'] != PIN or not oracle['sourceFilesChecked']:
        raise ValueError('Unverified original sources')
    records = oracle['records']
    if len({record['test'] for record in records}) != len(records):
        raise ValueError('Duplicate oracle identities')
    expected = {f'{mode} preserves original production boundaries: {record["test"]}'
                for record in records for mode in ['snapshot', 'workspace']}
    for check in CHECKS:
        for variant in ['positive', 'clean']:
            controls = [r for r in records if r['test'] == check + '-control-' + variant]
            if len(controls) != 1 or bool(controls[0]['expected']) != (variant == 'positive'):
                raise ValueError('Missing original positive or clean control: ' + check)
        expected.add('rejects a disabled configured detector in the fresh audit: ' + check)
    expected.update(['retains original configured positives and clean controls for all ten audited rules',
                     'preserves HNSW file-to-directory scope changes in one persistent workspace'])
    seen = set()
    for report in reports:
        if report['numFailedTests'] or report.get('numRuntimeErrorTestSuites', 0):
            raise ValueError('Fresh semantic audit failed')
        for suite in report['testResults']:
            for assertion in suite['assertionResults']:
                if assertion['status'] != 'passed':
                    continue
                name = assertion['fullName']
                if name in seen:
                    raise ValueError('Duplicate fresh assertion: ' + name)
                seen.add(name)
    if seen != expected:
        raise ValueError('Missing or extra fresh assertion identities: ' + str(seen ^ expected))
    return dict(inputs=len(records), positiveInputs=sum(bool(row['expected']) for row in records),
                apiComparisons=2 * len(records), disabledDetectors=10, disabledApiComparisons=20,
                persistentRevisions=4, assertions=len(seen), perRule=dict(Counter(row['check'] for row in records)))


def repository_delta(historical, original_path, selected_path, native_path):
    original, selected, native = [json.loads(path.read_text()) for path in [original_path, selected_path, native_path]]
    if digest(original_path) != historical['recordingSha256']:
        raise ValueError('Changed historical repository recording')
    if selected['commit'] != PIN or native['commit'] != PIN or selected['source_mismatches']:
        raise ValueError('Unverified repository sources')
    if selected['files'] != original['files'] or native['files'] != historical['files']:
        raise ValueError('Repository revalidation narrowed its file domain')
    if digest(selected_path) != native['recordingSha256']:
        raise ValueError('Changed revalidation recording')
    if native['nativeSha256'] != historical['engineSha256']:
        raise ValueError('A rule delta cannot certify a changed engine')
    current = {}
    for rule in historical['policyFileSha256']:
        path = Path('packs/data-layer/candidates/rules') / (rule + '.json')
        if not path.exists():
            path = Path('packs/data-layer/rules') / (rule + '.json')
        current[rule] = digest(path)
    changed = {rule for rule in current if current[rule] != historical['policyFileSha256'][rule]}
    if not changed or set(native['policyFileSha256']) != changed:
        raise ValueError('Repository revalidation must execute every changed rule')
    selected_checks = {rule.removeprefix('data-layer-').replace('-', '_') for rule in changed}
    if selected['events'] != [event for event in original['events'] if event['check'] in selected_checks]:
        raise ValueError('Changed original findings in repository revalidation')
    if native['policyFileSha256'] != {rule: current[rule] for rule in changed}:
        raise ValueError('Repository revalidation used stale rules')
    if any(native[key] for key in ['falsePositives', 'falseNegatives', 'failures', 'incrementalDifferences']):
        raise ValueError('Repository revalidation failed')
    expected = sum(len(event['findings']) for event in selected['events'])
    if native['expected'] != expected or native['actual'] != expected:
        raise ValueError('Repository revalidation changed diagnostic multiplicity')
    return dict(historicalUnchangedRules=len(current) - len(changed), revalidatedRules=sorted(changed),
                files=native['files'], expected=expected, actual=expected, falsePositives=0, falseNegatives=0,
                engineFailures=0, engineSha256=native['nativeSha256'], policyFileSha256=native['policyFileSha256'],
                measuredPhases=native['measuredPhases'], coldMs=native['coldMs'], compilationMs=native['compilationMs'],
                peakRssBytes=native['peakRssBytes'],
                artifactSha256={str(path): digest(path) for path in [original_path, selected_path, native_path]},
                note='The historical joint 100-rule measurement is unchanged. Only changed rules were rerun over its entire file domain. These are separate executions; their timings cannot be combined into a new 100-rule measurement. Parent CPU excludes producer processes. Positive detection is covered by the fresh source-backed regression corpus.')
