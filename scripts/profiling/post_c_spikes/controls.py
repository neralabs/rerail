"""Alternating fresh-process controls for explicit standalone spike artifacts."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess


def run(manifest, output):
    variants = json.loads(manifest.read_text())
    output.mkdir()
    env = dict(os.environ)
    for key in list(env):
        if key.startswith('RERAIL_PROFILE') or key in ['RERAIL_SQL_QUERY_PLANS', 'RERAIL_SQL_PROFILE_DIR', 'RERAIL_CODE_RULE_RELATIONAL_PROFILE']:
            env.pop(key)
    env['RERAIL_NATIVE_WORKERS'] = '4'
    trials = {variant['name']: [] for variant in variants}
    for trial in range(5):
        order = variants[trial % len(variants):] + variants[:trial % len(variants)]
        if trial % 2:
            order = list(reversed(order))
        for variant in order:
            stem = output / (variant['name'] + '-' + str(trial))
            env['RERAIL_ENGINE_PATH'] = str(Path(variant['addon']).resolve())
            command = ['node', '--import', 'tsx', 'conformance/data-layer/post-c-control.mjs', variant['recording'], variant['policies'], str(stem)]
            with stem.with_suffix('.stdout.log').open('w') as stdout, stem.with_suffix('.stderr.log').open('w') as stderr:
                subprocess.run(command, env=env, stdout=stdout, stderr=stderr, timeout=110, check=True)
            row = json.loads(stem.with_suffix('.json').read_text())
            assert row['passed']
            trials[variant['name']].append(row)
            print(variant['name'], trial, row['cpuMs'], row['maxRSSKiB'], flush=True)
    summary = {name: {'medianCpuMs': statistics.median(row['cpuMs'] for row in rows),
                      'cpuRangeMs': [min(row['cpuMs'] for row in rows), max(row['cpuMs'] for row in rows)],
                      'medianPeakRSSKiB': statistics.median(row['maxRSSKiB'] for row in rows),
                      'rssRangeKiB': [min(row['maxRSSKiB'] for row in rows), max(row['maxRSSKiB'] for row in rows)]}
               for name, rows in trials.items()}
    report = {'trials': trials, 'summary': summary,
              'protocol': 'Fresh process per trial. Five rotated/reversed trials per artifact. Imports precede CPU interval. Tracing, phase profiling, SQL capture, census and sampling are off. Node high-water RSS covers the complete process.',
              'variants': [{**variant, 'nativeSha256': hashlib.sha256(Path(variant['addon']).read_bytes()).hexdigest()} for variant in variants]}
    (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    run(args.manifest, args.output)
