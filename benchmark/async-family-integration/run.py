#!/usr/bin/env python3
"""Run the async comparison in separate processes, preserving failed gates."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser()
p.add_argument('--binary', required=True, type=Path)
p.add_argument('--corpus', required=True, type=Path)
p.add_argument('--out', required=True, type=Path)
p.add_argument('--stage', choices=['reference', 'rr-positive', 'rr', 'rr-profile', 'edges', 'producers'], required=True)
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
env = os.environ.copy()
# Each process has a declared cache boundary, independent of the developer's cache.
for name in ('RERAIL_PROFILE', 'RERAIL_SHARED_LIMIT', 'RERAIL_SAMPLE_EVERY', 'RERAIL_SCOPE_GUARDS', 'RERAIL_LIBRARIES', 'RERAIL_REQUESTS'):
    env.pop(name, None)
env.update(RUST_MIN_STACK='67108864', RERAIL_SHARED_CORPUS=str(a.corpus.resolve()),
           RERAIL_TAPE_CACHE=str((a.out / 'tape-cache.sqlite').resolve()),
           RERAIL_CORPUS_MEMO='off', RERAIL_CROSS_MEMO='off')
results = []

def run(name, test, extra=None, ignored=False):
    local = env | (extra or {})
    cmd = [str(a.binary.resolve()), test, '--nocapture', '--test-threads=1']
    if ignored:
        cmd.append('--ignored')
    started = time.time()
    with (a.out / (name + '.log')).open('w') as log:
        result = subprocess.run(cmd, env=local, stdout=log, stderr=subprocess.STDOUT)
    row = dict(name=name, command=cmd, exit_code=result.returncode, wall_seconds=time.time()-started,
               environment={k: v for k, v in local.items() if k.startswith('RERAIL_') or k == 'RUST_MIN_STACK'})
    results.append(row)
    (a.out / (a.stage + '-processes.json')).write_text(json.dumps(results, indent=2) + '\n')
    print(f'{name}: exit {result.returncode}, {row["wall_seconds"]:.2f}s', flush=True)
    return result.returncode

if a.stage == 'reference':
    run('admission', 'catalog_async_compiler_admission_is_accounted_for',
        {'RERAIL_ASYNC_ADMISSION_REPORT': str(a.out / 'admission.json')})
    run('reference-positive', 'async_reference::tests::',
        {'RERAIL_ASYNC_EXPORT_FIXTURES': str(a.out / 'edge-fixtures')})
    run('reference-full', 'async_reference::tests::complete_async_family_corpus_profile',
        {'RERAIL_SHARED_REPORT': str(a.out / 'reference-full.json')}, ignored=True)
elif a.stage == 'rr-positive':
    run('rr-positive', 'authored_rr_batched_positive_documents_preserve_all_four_labels',
        {'RERAIL_RR_POSITIVE_REPORT': str(a.out / 'rr-positive.json')})
elif a.stage in ('rr', 'rr-profile'):
    name = a.stage
    extra = {'RERAIL_PROF_CHUNKS': '1', 'RERAIL_RR_REPORT': str(a.out / (name + '.json')), 'RERAIL_RR_OUT': str(a.out / name)}
    if name == 'rr-profile':
        extra['RERAIL_PROFILE'] = str(a.out / name / 'profile')
    run(name, 'complete_async_family_authored_rr_corpus_profile', extra, ignored=True)
elif a.stage == 'producers':
    run('producers', 'complete_async_family_producer_profile',
        {'RERAIL_PRODUCER_REPORT': str(a.out / 'producers.json')}, ignored=True)
else:
    comparisons = []
    for fixture in sorted((a.out / 'edge-fixtures').iterdir()):
        name = 'edge-' + fixture.name
        code = run(name, 'complete_async_family_authored_rr_corpus_profile',
            {'RERAIL_SHARED_CORPUS': str(fixture), 'RERAIL_RR_REPORT': str(a.out / (name + '.json')),
             'RERAIL_RR_OUT': str(a.out / name)}, ignored=True)
        if code:
            continue
        expected = set(map(tuple, json.loads((fixture / 'expected.json').read_text())))
        actual = set(map(tuple, json.loads((a.out / (name + '.json')).read_text())['keys']))
        comparisons.append(dict(fixture=fixture.name, expected=len(expected), actual=len(actual),
                                missing=sorted(expected-actual), unexpected=sorted(actual-expected)))
    (a.out / 'edge-parity.json').write_text(json.dumps(comparisons, indent=2) + '\n')
    if any(x['missing'] or x['unexpected'] for x in comparisons):
        raise SystemExit('Authored .rr edge parity failed; see edge-parity.json')
raise SystemExit(int(any(row['exit_code'] for row in results)))
