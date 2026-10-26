#!/usr/bin/env python3
"""Run isolated, rotated cold-CPU trials. These are component tests, not acceptance runs."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

VARIANTS = {
    'anchor': ['broad', 'parent', 'child', 'chosen'],
    'fusion': ['staged', 'inline', 'materialized', 'fused'],
    'native': ['sql', 'scalar', 'batch'],
    'publication': ['file', 'packet-triggers', 'packet-set', 'packet-small-cache', 'packet-large-cache'],
    'planning': ['private-schema', 'reprepare', 'reuse', 'batch'],
    'composed': ['staged', 'fused', 'native', 'direct-scalar', 'direct-native'],
}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def run(args):
    root = Path(__file__).resolve().parents[3]
    binary = root / 'scripts/profiling/preparation/target/release/cold_cpu'
    args.output.mkdir(parents=True, exist_ok=False)
    sources = sorted((root / 'scripts/profiling/preparation/src/cold_cpu').glob('*.rs'))
    sources += [Path(__file__).resolve(), root / 'scripts/profiling/preparation/src/composition/mod.rs']
    sources += [root/'scripts/profiling/preparation/src/post_c/mod.rs',root/'scripts/profiling/preparation/Cargo.lock']
    sources += sorted((root/'native/src/code_rule_dsl/matcher/anchors/sql').rglob('*.rs'))
    sources += sorted((root/'native/src/code_rule_dsl/matcher/anchors/sql').glob('*.sql'))
    report = dict(format='rerail.cold-cpu-spike-trials/v1', acceptance=False,
                  experiment=args.experiment, scale=args.scale,
                  capture=str(args.capture.resolve()), captureSha256=digest(args.capture),
                  binarySha256=digest(binary), sources={str(p.relative_to(root)): digest(p) for p in sources},
                  revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
                  trials=[], complete=False,
                  qualifications=['Generic captured relation, not a complete linter or production-family rewrite.',
                                  'OS RSS is the process high-water mark through verification.',
                                  'Other diagnostic work may overlap; CPU, wall and RSS do not establish acceptance.'])
    started = time.monotonic()
    expected = None
    try:
        for trial in range(args.repetitions):
            variants = VARIANTS[args.experiment]
            order = variants[trial % len(variants):] + variants[:trial % len(variants)]
            for variant in order:
                output = args.output / f'{trial}-{variant}.json'
                command = [str(binary), str(args.capture.resolve()), args.experiment, variant, str(args.scale), str(output)]
                remaining = 110 - (time.monotonic() - started)
                if remaining <= 0:
                    raise TimeoutError('110-second group deadline')
                wall = time.monotonic()
                result = subprocess.run(command, text=True, capture_output=True, timeout=min(105, remaining))
                (args.output / f'{trial}-{variant}.log').write_text(result.stdout + result.stderr)
                if result.returncode:
                    raise RuntimeError(f'{variant} failed: {result.stderr[-1000:]}')
                data = json.loads(output.read_text())
                rows = data.pop('rows')
                if expected is None:
                    expected = rows
                elif rows != expected:
                    raise AssertionError(f'exact row multiset differs: {variant}')
                data.update(trial=trial, command=command, wallSeconds=time.monotonic()-wall,
                            rowCount=len(rows), rowSha256=hashlib.sha256(json.dumps(rows, separators=(',', ':')).encode()).hexdigest())
                report['trials'].append(data)
                print(f'{args.experiment} scale={args.scale} {variant} trial={trial}: {data["cpuMs"]:.3f} ms, rows={len(rows)}', flush=True)
        report['complete'] = True
    finally:
        report['elapsedSeconds'] = time.monotonic() - started
        (args.output / 'summary.json').write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    parser.add_argument('experiment', choices=VARIANTS)
    parser.add_argument('--scale', type=int, default=4)
    parser.add_argument('--repetitions', type=int, default=3)
    parser.add_argument('--output', type=Path, required=True)
    run(parser.parse_args())
