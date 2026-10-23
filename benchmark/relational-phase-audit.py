#!/usr/bin/env python3
"""Audit frozen B023 timing phases and enforce a preparation-specific target.

Reads saved measurements; does not rerun or replace the original qualification.
The new target is prepare CPU <= 2x reference, measured by sample medians.
"""
import argparse
import hashlib
import json
from pathlib import Path
from statistics import median


def audit(path):
    artifact = json.loads(path.read_text())
    frozen = artifact.get('frozenB023', artifact)
    result = frozen['result']
    samples = result['samples']
    assert len(samples) >= 3
    assert result['parity']
    phases = {}
    pairs = []
    for role in ['generated', 'reference']:
        reports = [s[role]['direct'] for s in samples]
        phases[role] = {
            'generationCpuNs': median(d['generationCpuNs'] for d in reports),
            'prepareCpuNs': median(d['metrics']['prepareCpuNs'] for d in reports),
            'executeCpuNs': median(d['metrics']['cpuNs'] for d in reports),
            'coldKernelCpuNs': median(d['coldKernelCpuNs'] for d in reports),
        }
    for sample in samples:
        generated, reference = (sample[role]['direct'] for role in ['generated', 'reference'])
        cold_gap = generated['coldKernelCpuNs'] - reference['coldKernelCpuNs']
        prepare_gap = generated['metrics']['prepareCpuNs'] - reference['metrics']['prepareCpuNs']
        pairs.append({
            'coldCpuGapNs': cold_gap, 'prepareCpuGapNs': prepare_gap,
            'prepareShareOfColdGap': prepare_gap / cold_gap if cold_gap > 0 else None,
        })
    target = 2 * phases['reference']['prepareCpuNs']
    return {
        'scope': 'Frozen B023 only; neither the original 818-clause workload nor the complete-job execution budget.',
        'artifactSha256': hashlib.sha256(path.read_bytes()).hexdigest(),
        'querySha256': frozen['querySha256'],
        'executableSha256': frozen['executableSha256'],
        'existingGates': result['gates'],
        'medianPhases': phases, 'pairedGapAttribution': pairs,
        'newPreparationGate': {
            'introducedBy': 'Post-phase validation audit; this was not a gate for the accepted change.',
            'maxPrepareCpuNs': target,
            'actualPrepareCpuNs': phases['generated']['prepareCpuNs'],
            'ratioToReference': phases['generated']['prepareCpuNs'] / phases['reference']['prepareCpuNs'],
            'passed': phases['generated']['prepareCpuNs'] <= target,
        },
        'limitations': [
            'Stage attribution identifies preparation as the main measured CPU gap, not its internal SQLite cause.',
            'Independent medians are not additive; gap shares use paired samples.',
            'Preparation and execution timers are disjoint thread CPU spans in whole_call/run.rs.',
            'The 2x preparation threshold is a new engineering target, not a conclusion derived from wrapper counts.',
            'An implementation must also preserve semantics and avoid execution-work regressions.',
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('artifact', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = audit(args.artifact)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    if not report['newPreparationGate']['passed']:
        raise SystemExit('Preparation gate failed; audit retained. Existing qualification is unchanged.')


if __name__ == '__main__':
    main()
