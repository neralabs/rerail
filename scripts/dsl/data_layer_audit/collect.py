#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Merge bounded upstream assertion runs and reject missing or duplicate coverage."""
import argparse
import copy
import json
from pathlib import Path


def collect(directory, mode, parts, controls=False):
    if controls:
        suites=[json.loads((directory/f'controls-{part}.json').read_text()) for part in range(parts)]
        merged=copy.deepcopy(suites[0]);merged['cases']={};merged['calls']=[];merged['exitCode']=0
        for suite in suites:
            for key in ('engineSha256','contractsSha256','policySha256','factPolicySha256'):
                if suite[key]!=merged[key]:raise ValueError('Inconsistent controls '+key)
            if suite['semanticFallbacks'] or merged['cases'].keys() & suite['cases'].keys():raise ValueError('Invalid controls')
            merged['cases'].update(suite['cases']);merged['calls'].extend(suite['calls']);merged['exitCode']|=suite['exitCode']
        expected=json.loads(Path('fixtures/host/data-layer-port/batch-08-case-coverage.json').read_text())
        if set(merged['cases'])!={row['test'] for row in expected['cases']} or merged['exitCode']!=1:raise ValueError('Incomplete or ineffective controls')
        failed={name for name,phases in merged['cases'].items() if any(row['outcome']=='failed' for row in phases)}
        rejected={call['check'] for call in merged['calls'] if call['test'] in failed and
                  (call['contract'] is None or call['contract'] in {'lazy-analyzed','cached-policy-comparison','memoized-policy-comparison'})}
        if len(rejected)!=9:raise ValueError('Controls must reject positives across all nine original suites')
        (directory/'native-pytest-positive-controls.json').write_text(json.dumps(merged,indent=2)+'\n')
        print(json.dumps(dict(controlCases=len(merged['cases']),exitCode=merged['exitCode'])))
        return
    api = [json.loads((directory/f'api-{mode}-{part}.json').read_text()) for part in range(parts)]
    suites = [json.loads((directory/f'api-{mode}-{part}.assertions.json').read_text()) for part in range(parts)]
    merged, assertions = copy.deepcopy(api[0]), copy.deepcopy(suites[0])
    merged['observations'], merged['differences'] = [], []
    assertions['cases'], assertions['calls'], assertions['instrumentationTranslations'] = {}, [], []
    assertions['originalCalls'] = {}
    for result, suite in zip(api, suites):
        if result['exitCode'] or result['differences'] or result['semanticFallbacks']:
            raise ValueError('An API comparison failed')
        if suite['exitCode'] or suite['semanticFallbacks']:
            raise ValueError('An original assertion run failed')
        for key in ('engineSha256', 'contractsSha256', 'policySha256', 'factPolicySha256'):
            if suite[key] != assertions[key]: raise ValueError('Inconsistent '+key)
        if assertions['cases'].keys() & suite['cases'].keys(): raise ValueError('Duplicate test coverage')
        assertions['cases'].update(suite['cases']); assertions['calls'].extend(suite['calls'])
        assertions['instrumentationTranslations'].extend(suite['instrumentationTranslations'])
        for key, count in suite['originalCalls'].items():
            assertions['originalCalls'][key] = assertions['originalCalls'].get(key, 0) + count
        merged['observations'].extend(result['observations'])
    if len(assertions['cases']) != 456 or len(merged['observations']) != 305:
        raise ValueError('Expected all 456 original cases and 305 complete results')
    expected=json.loads(Path('fixtures/host/data-layer-port/batch-08-case-coverage.json').read_text())
    if set(assertions['cases']) != {row['test'] for row in expected['cases']}:
        raise ValueError('The original test identities differ')
    assertions['sourceFilesChecked']=merged['sourceFilesChecked']={'perPart':[row['sourceFilesChecked'] for row in suites]}
    for phases in assertions['cases'].values():
        if len(phases) != 3 or any(row['outcome'] != 'passed' for row in phases):
            raise ValueError('An original test was skipped or incomplete')
    for name, value in [(f'full-api-final-{mode}.json', merged), (f'native-pytest-final-{mode}.json', assertions)]:
        (directory/name).write_text(json.dumps(value, indent=2)+'\n')
    print(json.dumps(dict(mode=mode, passed=len(assertions['cases']), completeResults=len(merged['observations']))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reports', type=Path, required=True)
    parser.add_argument('--mode', choices=['snapshot','workspace'], required=True)
    parser.add_argument('--parts', type=int, default=4)
    parser.add_argument('--controls',action='store_true')
    args = parser.parse_args(); collect(args.reports, args.mode, args.parts,args.controls)
