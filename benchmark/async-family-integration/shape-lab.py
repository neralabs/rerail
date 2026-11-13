#!/usr/bin/env python3
"""Controlled, test-binary-only comparison of composition and SQL physical shapes."""
import argparse
import json
import os
from pathlib import Path
import subprocess

p = argparse.ArgumentParser()
p.add_argument('--binary', type=Path, required=True)
p.add_argument('--corpus', type=Path, required=True)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--mode', choices=['replay', 'profiles', 'timings'], required=True)
p.add_argument('--cases', type=Path)
p.add_argument('--baseline-binary', type=Path, help='Compare this binary with --binary in timings mode, using composition in both')
a = p.parse_args()
if a.baseline_binary and a.mode != 'timings':
    p.error('--baseline-binary requires timings mode')
if a.mode == 'replay':
    if not a.cases or not a.cases.is_file():
        p.error('replay requires an existing --cases file')
    json.loads(a.cases.read_text())
a.out.mkdir(parents=True, exist_ok=True)
env = os.environ.copy()
for k in ['RERAIL_PROFILE', 'RERAIL_COMPOSE_LAB_NATIVE', 'RERAIL_COMPOSE_LAB_CASES', 'RERAIL_COMPOSE_LAB_OUT', 'RERAIL_RR_RULE_DIR']:
    env.pop(k, None)
env.update(RERAIL_SHARED_CORPUS=str(a.corpus.resolve()),
           RERAIL_TAPE_CACHE=str((a.out / 'tape-cache.sqlite').resolve()),
           RERAIL_CORPUS_MEMO='off', RERAIL_CROSS_MEMO='off', RUST_MIN_STACK='67108864')
records = []

def run(name, native=False, profile=False, cases=None, binary=None):
    local = env | dict(RERAIL_RR_OUT=str((a.out / name).resolve()),
                       RERAIL_RR_REPORT=str((a.out / (name + '.json')).resolve()))
    if native:
        local['RERAIL_COMPOSE_LAB_NATIVE'] = '1'
    if profile:
        local['RERAIL_PROFILE'] = str((a.out / name / 'profile').resolve())
    if cases:
        local['RERAIL_COMPOSE_LAB_CASES'] = str(cases.resolve())
        local['RERAIL_COMPOSE_LAB_OUT'] = str((a.out / name / 'cases').resolve())
    cmd = ['nice', '-n', '19', str((binary or a.binary).resolve()), 'complete_async_family_sql_shape_lab', '--ignored', '--nocapture', '--test-threads=1']
    with (a.out / (name + '.log')).open('w') as log:
        result = subprocess.run(cmd, env=local, stdout=log, stderr=subprocess.STDOUT)
    record = dict(name=name, command=cmd, exit_code=result.returncode,
                  environment={k:v for k,v in local.items() if k.startswith('RERAIL_') or k=='RUST_MIN_STACK'})
    if result.returncode == 0:
        d = json.loads((a.out / (name + '.json')).read_text())
        record.update(cpu=d['cpuNs']/1e9, wall=d['wallNs']/1e9, prepared=d['report']['prepared'],
                      files=d['report']['files'], keys=d['keys'], phases=d['report']['phases'])
    records.append(record)
    (a.out / (a.mode + '-processes.json')).write_text(json.dumps(records, indent=2)+'\n')
    print(name, result.returncode, record.get('cpu'), record.get('wall'), flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)

if a.mode == 'replay':
    run('replay', cases=a.cases)
elif a.mode == 'profiles':
    run('warmup')
    run('native-profile', native=True, profile=True)
    run('composed-profile', profile=True)
else:
    run('warmup')
    if a.baseline_binary:
        run('warmup-baseline', binary=a.baseline_binary)
    warmups = len(records)
    for i, native in enumerate([True, False, False, True, True, False]):
        if a.baseline_binary:
            run(f'{i}-' + ('before' if native else 'fixed'), binary=a.baseline_binary if native else a.binary)
        else:
            run(f'{i}-' + ('native' if native else 'composed'), native=native)
    measured = records[warmups:]
    assert all(r['keys'] == measured[0]['keys'] and r['files'] == measured[0]['files'] for r in measured)
