"""Replay all frozen candidate domains with complete multiset comparison."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess

from candidates import covered_input, rewrite


def run(raw, host, output):
    output.mkdir()
    captures = sorted(raw.glob('candidate-*.json'))
    assert captures
    prepared = {mode: [] for mode in ['baseline', 'folded', 'private']}
    for index, capture in enumerate(captures):
        baseline = output / 'baseline' / str(index) / 'capture.json'
        covered_input(capture, baseline)
        prepared['baseline'].append(baseline)
        for mode in ['folded', 'private']:
            target = output / mode / str(index) / 'capture.json'
            rewrite(baseline, target, mode == 'private')
            prepared[mode].append(target)
    env = {key: value for key, value in os.environ.items()
           if not key.startswith('RERAIL_PROFILE') and key != 'RERAIL_SQL_PROFILE_DIR'}
    trials = []
    truth = {}
    for trial in range(5):
        modes = ['baseline', 'folded', 'private']
        modes = modes[trial % 3:] + modes[:trial % 3]
        if trial % 2:
            modes.reverse()
        for mode in modes:
            reports = []
            for index, capture in enumerate(prepared[mode]):
                target = output / f'{mode}-{trial}-{index}.json'
                command = [str(host), str(capture), '1024', '512', str(target)]
                with target.with_suffix('.log').open('w') as log:
                    subprocess.run(command, env=env, stdout=log, stderr=log, timeout=110, check=True)
                rows = sorted(json.loads(target.with_suffix('.rows.json').read_text()))
                if mode == 'baseline':
                    if index in truth:
                        assert rows == truth[index]
                    truth[index] = rows
                else:
                    assert rows == truth[index], (mode, trial, index)
                reports.append(json.loads(target.read_text()))
            trials.append({'mode': mode, 'trial': trial,
                           'statements': sum(row['executionStatements'] for row in reports),
                           'cpuMs': sum(row['sqlCpuMs'] for row in reports),
                           'prepareCpuMs': sum(row['sqliteCompileCpuMs'] for row in reports),
                           'peakSqliteStepBytes': max(value['peakBytes'] for row in reports for value in row['largestHeapPeaks']),
                           'maxProcessRSSBytes': max(row['processPeakRssBytes'] for row in reports),
                           'multisetsMatch': True})
    summary = {}
    for mode in prepared:
        rows = [row for row in trials if row['mode'] == mode]
        summary[mode] = {'statements': rows[0]['statements'],
                         'medianCpuMs': statistics.median(row['cpuMs'] for row in rows),
                         'cpuRangeMs': [min(row['cpuMs'] for row in rows), max(row['cpuMs'] for row in rows)],
                         'peakSqliteStepBytes': max(row['peakSqliteStepBytes'] for row in rows)}
    report = {'format': 'rerail.post-c-candidate-suite/v1', 'summary': summary, 'trials': trials,
              'scope': 'Four frozen candidate domains after the already completed structural phase. Includes candidate setup, requests, stage/result statements, retirement and transactions. Source publication and structural preparation are outside this component replay. No complete-job saving is inferred.'}
    (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('raw', type=Path)
    parser.add_argument('host', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    run(args.raw, args.host, args.output)
