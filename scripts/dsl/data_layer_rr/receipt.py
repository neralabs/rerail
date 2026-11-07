"""Bundle complete direct-suite evidence without claiming full migration acceptance."""
import argparse
from collections import Counter
import gzip
import json
from pathlib import Path

from migration import ROOT, PIN, sha, read_json


def validate(baseline, reports, inventory, engine_sha):
    if baseline['commit'] != PIN or baseline['pytest_exit_code'] or baseline.get('source_mismatches'):
        raise ValueError('Source recording is not a passing pinned run')
    if not reports or len(reports) != reports[0]['parts'] or {r['part'] for r in reports} != set(range(len(reports))):
        raise ValueError('Direct suite shards are incomplete or duplicated')
    collected = reports[0]['collected']
    if len(collected) != len(set(collected)) or len(collected) != baseline['collected_tests']:
        raise ValueError('Original case collection is incomplete')
    files = {name for name in reports[0]['sourceFiles'] if name.startswith('scripts/tests/')}
    wanted = {identity for identity in inventory['cases'] if identity.split('::', 1)[0] in files}
    if set(collected) != wanted:
        raise ValueError('Direct suite collection differs from the independent original inventory')
    seen, observed, calls, no_native = set(), Counter(), [], []
    for report in reports:
        if (report['sourcePin'] != PIN or report['oracle'] is not False or report['exitCode'] or
                report['engineSha256'] != engine_sha or report['collected'] != collected or
                report['semanticFallbacks'] or report['missingResults']):
            raise ValueError('A direct suite failed, omitted calls, or used another engine or corpus')
        expected = set(collected[report['part']::report['parts']])
        if set(report['cases']) != expected or seen & expected:
            raise ValueError('Direct suite cases are missing or duplicated')
        seen.update(expected)
        for phases in report['cases'].values():
            if {r['phase']: r['outcome'] for r in phases} != dict(setup='passed', call='passed', teardown='passed'):
                raise ValueError('An original assertion failed or was skipped')
        for item in report['detectorObservations']:
            if item.get('expectedSourceResult') != item.get('actualSourceResult'):
                raise ValueError('A source API return differs')
            observed[(item['test'], item['check'], item['entry'])] += 1
        if any(call['status'] != 'passed' for call in report['calls']):
            raise ValueError('A native call failed')
        calls.extend(report['calls'])
        no_native.extend(report['casesWithoutNativeCalls'])
    expected = Counter((r['test'], r['check'], r['entry']) for r in baseline['events'] if r['test'] in wanted)
    if observed != expected:
        raise ValueError('Original API returns are missing or duplicated')
    return dict(cases=len(seen), nativeCalls=len(calls), losslessApiReturns=sum(observed.values()),
        casesWithoutNativeCalls=no_native,
        nativeCallsWithRows=sum(bool(call['diagnostics']) for call in calls),
        nativeCallsWithRowsDefinition='Includes helper Boolean probes; this is not a detector-positive case count')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--reports', type=Path, nargs='+', required=True)
    parser.add_argument('--mutations', type=Path, nargs='+', required=True)
    parser.add_argument('--engine-info', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    baseline, reports = read_json(args.baseline), [read_json(p) for p in args.reports]
    engine = read_json(args.engine_info)
    if sha((ROOT / engine['enginePath']).read_bytes()) != engine['engineSha256']:
        raise ValueError('Frozen executable differs from its build receipt')
    inventory_path = ROOT / 'fixtures/host/data-layer-port/rr-original-test-inventory.json'
    counts = validate(baseline, reports, read_json(inventory_path), engine['engineSha256'])
    checks = {call['check'] for report in reports for call in report['calls']}
    mutations = [read_json(p) for p in args.mutations]
    if len(mutations) != len(checks) or {r['check'] for r in mutations} != checks or any(not r['rejected'] for r in mutations):
        raise ValueError('Each direct policy requires its own positive mutation control')
    for report in [*reports, *[m['report'] for m in mutations]]:
        for name, digest in report['harnessFiles'].items():
            if sha((ROOT / name).read_bytes()) != digest:
                raise ValueError('Stale harness: ' + name)
        for call in report['calls']:
            if not report['exitCode']:
                for item in call['programInputs']:
                    if sha((ROOT / item['path']).read_bytes()) != item['sha256']:
                        raise ValueError('Stale policy or library: ' + item['path'])
    result = dict(format='rerail.rr-direct-source-contracts/v2', scope='Eleven direct original modules; snapshot execution only',
        fullMigrationAcceptance=False, sourcePin=PIN, **engine, **counts,
        originalInventory=dict(path=inventory_path.relative_to(ROOT).as_posix(), sha256=sha(inventory_path.read_bytes())),
        originalRecordingSha256=sha(args.baseline.read_bytes()), originalRecording=baseline, reports=reports,
        positiveMutationControls=mutations,
        outstanding='Associated suites, portable helper counterparts, configured reporting and persistent workspace contracts remain required')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(gzip.compress(json.dumps(result, sort_keys=True, ensure_ascii=False).encode(), mtime=0))
    print(json.dumps({key: result[key] for key in ['cases', 'nativeCalls', 'losslessApiReturns', 'fullMigrationAcceptance']}))


if __name__ == '__main__':
    main()
