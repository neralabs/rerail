"""Prove that a disabled policy fails an unchanged original positive assertion."""
import argparse
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile

from migration import ROOT, PIN, read_json, sha, write_json

DISABLED = '.policy "disabled-control" revision 1\n.language python\n.violation Disabled(at: span) "disabled"\nDisabled(@place("input.py", 1)) :- 1 = 0.\n'


def auxiliary_output(check, row):
    """Only these transport outputs can remain in a disabled backend detector."""
    return check == 'backend_coupling' and row['code'] in {'BC_CONFIG', 'BC_CHECK', 'BC_FILE'}


def disable_policy(check, source):
    if check != 'backend_coupling':
        return DISABLED
    disabled, count = re.subn(r'(?m)^(Coupling\([^\n]*?\)\s*:-)', r'\1 1 = 0,', source)
    if count != 1:
        raise ValueError('The backend mutation requires exactly one detector output clause')
    return disabled


def run(args):
    baseline, collection = read_json(args.baseline), read_json(args.collection)
    if baseline['commit'] != PIN or collection['sourcePin'] != PIN or baseline['pytest_exit_code']:
        raise ValueError('A passing pinned source recording and an independent case list are required')
    positives = [event for event in baseline['events'] if event['check'] == args.check and event.get('findings')]
    if not positives:
        raise ValueError('The source recording has no detector-positive case for ' + args.check)
    identity = positives[0]['test']
    identities = collection['collected']
    if (collection.get('originalRecordingSha256') != sha(args.baseline.read_bytes()) or
            len(identities) != baseline['collected_tests'] or len(set(identities)) != len(identities)):
        raise ValueError('Collection is incomplete or belongs to another source recording')
    if identities.count(identity) != 1:
        raise ValueError('Positive case is missing or duplicated in the full collection')
    index = identities.index(identity)
    with tempfile.TemporaryDirectory(prefix='rr-disabled-', dir=ROOT / '.rerail') as temporary:
        root = Path(temporary)
        # Collection still imports every original module. Only the chosen positive case executes.
        for check in args.checks.split(','):
            filename = check.replace('_', '-') + '.rr'
            source = (ROOT / 'packs/data-layer/candidates/rr' / filename).read_text()
            (root / filename).write_text(disable_policy(check, source))
        disabled = (root / (args.check.replace('_', '-') + '.rr')).read_text()
        report_path = root / 'report.json'
        command = [sys.executable, str(Path(__file__).with_name('suite.py')), '--source', str(args.source.resolve()),
            '--engine', str(args.engine.resolve()), '--checks', args.checks, '--rule-root', str(root),
            '--baseline', str(args.baseline.resolve()), '--parts', str(len(identities)), '--part', str(index),
            '--out', str(report_path)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=105,
            env={**os.environ, 'PYTHONHASHSEED': '0'})
        if result.returncode != 1 or not report_path.is_file():
            raise AssertionError('Mutation did not produce an assertion failure\n' + result.stdout + result.stderr)
        report = read_json(report_path)
        if set(report['cases']) != {identity} or report['collected'] != identities:
            raise AssertionError('Mutation selected another original assertion')
        if report['semanticFallbacks']:
            raise AssertionError('Mutation failed through a source fallback')
        # The expected first positive failure can prevent later calls in this same test body.
        # Ordinary passing runs still require every call; mutation reports retain the unreached tail.
        if any(row['test'] != identity or row['check'] != args.check or row['count'] <= 0
               for row in report['missingResults']):
            raise AssertionError('Mutation omitted an unrelated source observation')
        phases = {phase['phase']: phase['outcome'] for phase in report['cases'][identity]}
        if phases != dict(setup='passed', call='failed', teardown='passed'):
            raise AssertionError('Mutation failed outside the original test body')
        if not report['calls'] or any(call['status'] != 'passed' or
                any(not auxiliary_output(args.check, row) for row in call['diagnostics']) for call in report['calls']):
            raise AssertionError('Mutation must execute a successful, empty native detector')
        observations = [row for row in report['detectorObservations'] if row['test'] == identity and row['check'] == args.check]
        if not observations or all(row.get('actualSourceResult') == row.get('expectedSourceResult') for row in observations):
            raise AssertionError('Disabled detector was not rejected by the complete source API comparison')
        return dict(format='rerail.rr-original-positive-mutation/v1', sourcePin=PIN, check=args.check,
            test=identity, rejected=True, reason='Original positive test rejects empty native results through the complete source API comparison',
            expectedUnreachedCalls=report['missingResults'],
            disabledPolicy=disabled, disabledPolicySha256=sha(disabled.encode()),
            baselineSha256=sha(args.baseline.read_bytes()), collectionSha256=sha(args.collection.read_bytes()),
            report=report, pytestOutput=result.stdout + result.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--collection', type=Path, required=True)
    parser.add_argument('--checks', required=True)
    parser.add_argument('--check', required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    result = run(args)
    result['harnessSha256'] = sha(Path(__file__).read_bytes())
    write_json(args.out, result)
    print(json.dumps(dict(check=args.check, test=result['test'], rejected=True)))


if __name__ == '__main__':
    main()
