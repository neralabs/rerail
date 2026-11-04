#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Bind native pytest outcomes to the tested rules, contracts, engine and source."""
import argparse
import hashlib
import json
from collections import Counter,defaultdict
from pathlib import Path


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def summarize(path):
    report=json.loads(path.read_text())
    if report['exitCode'] or report['semanticFallbacks']:raise ValueError('Native assertions did not pass without fallback')
    cases={}
    calls=defaultdict(list)
    for call in report['calls']:calls[call['test']].append(call)
    for name,phases in report['cases'].items():
        if len(phases)!=3 or any(phase['outcome']!='passed' for phase in phases):raise ValueError('Incomplete original assertion: '+name)
        invoked=calls[name]
        cases[name]=dict(outcome='passed',nativeDiagnosticCalls=sum(call['contract'] is None for call in invoked),
          nativePositiveDiagnosticCalls=sum(call['contract'] is None and call['findings']>0 for call in invoked),
          nativeHelperCalls=sum(call['contract'] is not None for call in invoked),
          contracts=sorted({call['contract'] for call in invoked if call['contract']}))
    if len(cases)!=456:raise ValueError('The complete pinned batch-eight suite must contain 456 cases')
    for check,digest in report['policySha256'].items():
        if digest!=sha(Path('packs/data-layer/candidates/rules/data-layer-'+check.replace('_','-')+'.json')):raise ValueError('Stale policy receipt: '+check)
    return dict(reportSha256=sha(path),collected=len(cases),passed=len(cases),failed=0,skipped=0,
      sourceFilesChecked=report['sourceFilesChecked'],semanticFallbacks=report['semanticFallbacks'],
      nativeCalls=len(report['calls']),casesWithNativeCalls=sum(bool(calls[name]) for name in cases),
      originalHostCalls=report['originalCalls'],instrumentationTranslations=report['instrumentationTranslations'],cases=cases)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine-receipt',type=Path,default=Path('fixtures/host/data-layer-port/batch-08-engine.json'))
    parser.add_argument('--reports',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();paths={mode:args.reports/f'native-pytest-final-{mode}.json' for mode in ('snapshot','workspace')}
    originals={mode:json.loads(path.read_text()) for mode,path in paths.items()}
    snapshot=originals['snapshot'];workspace=originals['workspace']
    engine=json.loads(args.engine_receipt.read_text())
    if engine['native_sha256']!=snapshot['engineSha256']:raise ValueError('Engine receipt differs from tested native addon')
    for key in ('engineSha256','contractsSha256','policySha256'):
        if snapshot[key]!=workspace[key]:raise ValueError('Execution paths used different '+key)
    controls_path=args.reports/'native-pytest-positive-controls.json';controls=json.loads(controls_path.read_text())
    failed={name for name,phases in controls['cases'].items() if any(phase['outcome']=='failed' for phase in phases)}
    rejected=defaultdict(set)
    for call in controls['calls']:
        if call['contract'] is None and call['test'] in failed:rejected[call['check']].add(call['test'])
    if controls['exitCode']==0 or controls['semanticFallbacks'] or len(rejected)!=9:raise ValueError('Disabled detectors must fail the original positive assertions for all nine suites')
    report=dict(format='rerail.data-layer-native-assertions/v1',sourceCommit='2b1504adc4291daaf080b2e932962b134da1a69a',
      engineCommit=engine['commit'],engineSha256=snapshot['engineSha256'],contractsSha256=snapshot['contractsSha256'],policySha256=snapshot['policySha256'],
      implementationSha256={str(path):sha(path) for path in sorted([*Path('scripts/dsl/data_layer_audit').glob('native_*.py'),Path('conformance/data-layer/native-worker.ts')])},
      executionPaths={mode:summarize(path) for mode,path in paths.items()},
      positiveControls=dict(reportSha256=sha(controls_path),exitCode=controls['exitCode'],failedCases=len(failed),
        rejectedByCheck={check:sorted(cases) for check,cases in sorted(rejected.items())},
        sqlSurface='No original pytest module at the source pin; checked by independent oracle probes and the native always-clean mutation test.'),
      boundaries=['Original source files and behavior assertions are unchanged.',
        'Three instrumentation functions (four parametrized cases) translate Python hooks to actual native traversal/materialization counters.',
        'Host signing orchestration, file discovery, constants and DTO formatting remain Python; original detector and analyzer execution is forbidden.',
        'No full-repository cold execution was run.'])
    args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({mode:{key:value for key,value in row.items() if key in ('collected','passed','failed','skipped','nativeCalls','casesWithNativeCalls')} for mode,row in report['executionPaths'].items()}))


if __name__=='__main__':main()
