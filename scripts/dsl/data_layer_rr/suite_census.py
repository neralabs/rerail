"""Measure original-suite coverage on one native engine from its assertion reports."""
import argparse
from collections import Counter
from pathlib import Path

from migration import MANIFEST, PIN, ROOT, fixtures_path, read_json, sha, write_json


MODES = ('snapshot', 'workspace')
PASSED = dict(setup='passed', call='passed', teardown='passed')


def census(manifest, inventory, reports):
    if inventory['sourcePin'] != PIN:
        raise ValueError('Original-suite inventory uses another source revision')
    engine = manifest['engineSource']['cliArtifact']
    rules = {rule['check']: rule for rule in manifest['rules']}
    cases = {module: set() for module in inventory['files']}
    for identity in inventory['cases']:
        cases[identity.split('::', 1)[0]].add(identity)

    outcomes = {mode: {} for mode in MODES}
    calls = Counter()
    observations = Counter()
    report_failures = []
    for report in reports:
        mode = report['mode']
        if (mode not in outcomes or report['oracle'] is not False or
                report['sourcePin'] != PIN or report['engineSha256'] != engine['sha256']):
            raise ValueError('Original-suite reports must use the same pinned native engine')
        for check, digest in report['policySha256'].items():
            if rules[check]['ruleSha256'] != digest:
                raise ValueError(f'{check}: report uses another policy revision')
        if report['exitCode'] or report['missingResults'] or report['semanticFallbacks']:
            report_failures.append(dict(mode=mode, part=report['part'], testTargets=report['testTargets'],
                exitCode=report['exitCode'], missingResults=report['missingResults'],
                semanticFallbacks=report['semanticFallbacks']))
        reported = set()
        for identity, phases in report['cases'].items():
            module = identity.split('::', 1)[0]
            if identity not in cases.get(module, set()) or report['sourceFiles'].get(module) != inventory['files'][module]['sha256']:
                raise ValueError(f'{identity}: report uses another original test')
            row = {p['phase']: p['outcome'] for p in phases}
            for check in report['policySha256'].keys() & set(inventory['files'][module]['detectors']):
                key = (check, identity)
                if key in outcomes[mode]:
                    raise ValueError(f'{identity}: duplicate result for {check} in {mode}')
                outcomes[mode][key] = row
                reported.add(key)
        for call in report['calls']:
            key = (call['check'], call['test'])
            if key in reported and call['status'] == 'passed':
                calls[mode, *key] += 1
        for row in report['detectorObservations']:
            key = (row['check'], row['test'])
            if (key in reported and row.get('status') == 'passed' and row.get('expected') and
                    row.get('expected') == row.get('actual') and
                    'expectedSourceResult' in row and 'actualSourceResult' in row and
                    row['expectedSourceResult'] == row['actualSourceResult']):
                observations[mode, *key] += 1

    results = []
    for check, rule in rules.items():
        modules = sorted(name for name, info in inventory['files'].items() if check in info['detectors'])
        coverage = {}
        for mode in MODES:
            per_module = []
            for module in modules:
                seen = {case: outcomes[mode][check, case] for case in cases[module] if (check, case) in outcomes[mode]}
                failures = sorted(case for case, phases in seen.items() if phases != PASSED)
                passed = set(seen) - set(failures)
                per_module.append(dict(module=module, originalCases=len(cases[module]),
                    passed=len(passed), failed=failures, pending=len(cases[module] - seen.keys()),
                    passedWithNativeCalls=sum(bool(calls[mode, check, case]) for case in passed),
                    matchedPositiveObservations=sum(observations[mode, check, case] for case in passed)))
            coverage[mode] = per_module
        results.append(dict(check=check, translation=rule['status'], originalSuite=coverage))
    return dict(format='rerail.rr-original-suite-census/v1', engine=engine, sourcePin=PIN,
                reports=len(reports), reportFailures=report_failures, checks=results)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reports', type=Path, required=True, nargs='+')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    inventory = fixtures_path('host', 'data-layer-port', 'rr-original-test-inventory-v2.json')
    manifest = read_json(ROOT / MANIFEST)
    result = census(manifest, read_json(inventory), [read_json(path) for path in args.reports])
    result['inventorySha256'] = sha(inventory.read_bytes())
    result['reportSha256'] = {str(path): sha(path.read_bytes()) for path in args.reports}
    write_json(args.out, result)


if __name__ == '__main__':
    main()
