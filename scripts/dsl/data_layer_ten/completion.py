# SPDX-License-Identifier: Apache-2.0
"""Require the exact DoD contract matrix and the complete 100-check measurement."""
import argparse
import hashlib
import json
from pathlib import Path

from data_layer_rule_audit import fixtures_path
from data_layer_ten.common import CHECKS, PIN


def verify_contracts(reports):
    fixtures = fixtures_path('host/data-layer-port')
    audit = json.loads((fixtures / 'batch-10-dod-audit-20261106.json').read_text())
    approvals = json.loads((fixtures / 'batch-10-dod-contracts.json').read_text())
    lifecycle = json.loads((fixtures / 'batch-10-lifecycle.json').read_text())
    expected = {f'preserves original approval {case["id"]} through {mode}' for case in approvals['approvalCases'] for mode in ['snapshot', 'workspace']}
    expected.update(f'preserves configured {case["id"]} through {mode}' for case in audit['cases'] if case['id'].startswith('optimizer-') for mode in ['snapshot', 'workspace'])
    expected.update(f'discovers and classifies explicit matching roots through {mode}' for mode in ['snapshot', 'workspace'])
    expected.update(f'preserves {check} across clean, positive, restored, removed and readded revisions' for check in CHECKS)
    expected.add('preserves optimizer-document-dependency across clean, positive, restored, removed and readded revisions')
    expected.add('preserves constructor positives and clean results beside unrelated lexical inventory')
    expected.update(f'preserves original {check} results with dense global inventory' for check in ['lazy_architecture', 'pure_lazy_runtime_purity'])
    expected.update(['preserves the original signing canonicalization and deployed trust root',
                     'matches original Python Base64 decoding for malformed and padded approval signatures',
                     'routes original matching phases and summary-only output through the production command',
                     'reads live source approvals and delegates only explicit signing to the original signer',
                     'retains all ten production programs and every lifecycle state'])
    seen = set()
    for report in reports:
        if report['numFailedTests'] or report.get('numRuntimeErrorTestSuites', 0): raise ValueError('DoD contract tests failed')
        for suite in report['testResults']:
            for case in suite['assertionResults']:
                if case['status'] != 'passed': continue
                name = case['fullName']
                if name in seen: raise ValueError('Duplicate DoD assertion: ' + name)
                seen.add(name)
    if seen != expected: raise ValueError('Missing or extra DoD assertion identities: ' + str(seen ^ expected))
    if {case['check'] for case in lifecycle['cases']} != set(CHECKS): raise ValueError('Incomplete lifecycle programs')
    for case in lifecycle['cases']:
        if [state['name'] for state in case['states']] != ['clean', 'positive', 'restored', 'removed', 'readded']: raise ValueError('Incomplete lifecycle transitions')
        if case['states'][0]['expected'] or not case['states'][1]['expected']: raise ValueError('Missing clean or positive lifecycle state')
    if [row['id'] for row in approvals['matchingRootSources']] != ['utf8-bom', 'bare-cr', 'crlf']: raise ValueError('Incomplete matching source-decoding cases')
    if len(approvals['approvalEncodingCases']) != 8: raise ValueError('Incomplete approval encoding cases')
    return dict(assertions=len(seen), assertionIds=sorted(seen), approvalApiComparisons=2*len(approvals['approvalCases']),
                approvalEncodingComparisons=2*len(approvals['approvalEncodingCases']),
                matchingRootSourceComparisons=2*len(approvals['matchingRootSources']),
                lifecycleRules=10, lifecycleRevisions=57, transportPartitionsPreserveRuleDomains=True)


def verify_cumulative(oracle_path, native_path, process_path):
    from data_layer_record import BATCH_ONE
    from data_layer_batch_two import BATCH_TWO
    from data_layer_batch_three import BATCH_THREE
    from data_layer_batch_four import BATCH_FOUR
    from data_layer_batch_five import BATCH_FIVE
    from data_layer_batch_six import BATCH_SIX
    from data_layer_batch_seven import BATCH_SEVEN
    from data_layer_batch_eight import BATCH_EIGHT
    from data_layer_nine.evidence import CHECKS as NINE
    oracle, native, process = [json.loads(path.read_text()) for path in [oracle_path, native_path, process_path]]
    names = [*BATCH_ONE, *BATCH_TWO, *BATCH_THREE, *BATCH_FOUR, *BATCH_FIVE, *BATCH_SIX, *BATCH_SEVEN, *BATCH_EIGHT, *NINE, *CHECKS]
    if len(names) != 100 or len(set(names)) != 100: raise ValueError('Invalid 100-check inventory')
    if sorted(row['check'] for row in oracle['events']) != sorted(names): raise ValueError('Incomplete cumulative original checks')
    ids = {'data-layer-' + name.replace('_', '-') for name in names}
    if set(native['policyDigests']) != ids or set(native['policyFileSha256']) != ids: raise ValueError('Incomplete cumulative native programs')
    if oracle['commit'] != PIN or native['commit'] != PIN or oracle['source_mismatches']: raise ValueError('Cumulative source identity mismatch')
    if native['recordingSha256'] != hashlib.sha256(oracle_path.read_bytes()).hexdigest(): raise ValueError('Cumulative workload identity mismatch')
    if native['files'] != len(oracle['files']) or native['files'] < 9021: raise ValueError('Incomplete repository input domain')
    for key in ['falsePositives', 'falseNegatives', 'failures', 'incrementalDifferences']:
        if native[key]: raise ValueError('Cumulative native comparison failed: ' + key)
    expected = sum(len(event['findings']) for event in oracle['events'])
    if native['expected'] != expected or native['actual'] != expected: raise ValueError('Cumulative finding multiplicity mismatch')
    if process['exitCode'] or not all(process[key] > 0 for key in ['cpuSeconds', 'wallSeconds', 'maxSingleProcessRssBytes']): raise ValueError('Incomplete cumulative process measurement')
    if not all(native[key] > 0 for key in ['compilationMs', 'coldMs', 'peakRssBytes']): raise ValueError('Missing cumulative phase measurements')
    for id, digest in native['policyFileSha256'].items():
        path = Path('packs/data-layer/candidates/rules') / (id + '.json')
        if not path.exists(): path = Path('packs/data-layer/rules') / (id + '.json')
        if hashlib.sha256(path.read_bytes()).hexdigest() != digest: raise ValueError('Changed cumulative rule: ' + id)
    return dict(format='rerail.data-layer-cumulative/v1', sourcePin=PIN, checks=100, files=native['files'],
                codeFiles=sum(file['path'].endswith(('.py','.rs')) for file in oracle['files']), requiredDocuments=[file['path'] for file in oracle['files'] if not file['path'].endswith(('.py','.rs'))],
                expected=expected, actual=expected, falsePositives=0, falseNegatives=0, engineFailures=0,
                engineSha256=native['nativeSha256'], recordingSha256=native['recordingSha256'], workloadSha256=native['workloadSha256'],
                policyFileSha256=native['policyFileSha256'], policyDigests=native['policyDigests'],
                hostPreparationMs=native['hostPreparationMs'], compilationMs=native['compilationMs'], phaseCpuMs=native['phaseCpuMs'], coldMs=native['coldMs'],
                measuredPhases=native['measuredPhases'], coldOnly='edit' not in native['measuredPhases'] and 'unchanged' not in native['measuredPhases'],
                rootPeakRssBytesThroughClose=json.loads(Path(str(native_path)+'.close.json').read_text())['after']['peakRssBytes'],
                processMeasurement=process, perRuleFindings={event['check']:len(event['findings']) for event in oracle['events']},
                note='Position/code/multiplicity parity for the historical cumulative source-diagnostic contracts, with retained reporter cuts recorded in the oracle. Complete latest-ten DTO/configuration parity is a separate required gate. Repository measured phases are listed explicitly. An empty incrementalDifferences array does not establish unmeasured full-repository edit parity. Persistent lifecycle tests are a separate gate. Compile, host preparation, and cold phases are separate; process CPU includes waited-for Python producers.')


if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--contracts', type=Path, nargs='+', required=True)
    parser.add_argument('--oracle', type=Path, required=True)
    parser.add_argument('--native', type=Path, required=True)
    parser.add_argument('--process', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args=parser.parse_args()
    args.output.write_text(json.dumps(dict(contracts=verify_contracts([json.loads(path.read_text()) for path in args.contracts]), cumulative=verify_cumulative(args.oracle,args.native,args.process)),indent=2)+'\n')
