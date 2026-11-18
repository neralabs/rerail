#!/usr/bin/env python3
"""Summarize same-source SQL experiments without counting captured inputs as free production."""
import argparse
import json
from pathlib import Path
import re

p = argparse.ArgumentParser()
p.add_argument('directory', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
results = {}
for source in sorted(a.directory.rglob('runs.jsonl')):
    groups = {}
    for line in source.read_text().splitlines():
        row = json.loads(line)
        key = row['case']
        d = groups.setdefault(key, dict(runs=0, cpu=0, wall=0, prepareCpu=0, vm=0, scans=0,
                                       sorts=0, autoindex=0, reprepare=0, rows=0, failures=[],
                                       parity_checks=0, parity_failures=0, root_count=0, inputs={}))
        for key in ['cpu', 'wall', 'prepareCpu', 'vm', 'scans', 'sorts', 'autoindex', 'reprepare', 'rows']:
            d[key] += row[key]
        d['runs'] += 1
        d['parity_checks'] += row.get('parity') is not None
        d['parity_failures'] += row.get('parity') is False
        if row['error']:
            d['failures'].append(row['error'])
        if 'result' in row:
            d['root_count'] += sum(map(int, re.findall(r'Integer\((\d+)\)', row['result'])))
        for rel, counts in row['inputs'].items():
            total = d['inputs'].setdefault(rel, [0, 0, 0, 0])
            for i, value in enumerate(counts):
                total[i] += value
    results[str(source.relative_to(a.directory))] = groups
    print(source)
    for name, d in groups.items():
        print(f'{name:42} {d["vm"]:10,d} VM  {d["cpu"]:.4f}s CPU  {d["autoindex"]:8,d} auto-index rows  {d["parity_failures"]} parity failures  {len(d["failures"])} errors')
a.output.write_text(json.dumps(dict(scope='Snapshot capture costs are separate; root costs are inclusive and non-additive.', runs=results), indent=2)+'\n')
if any(d['parity_failures'] for groups in results.values() for d in groups.values()):
    raise SystemExit('A shape experiment changed the result rows.')
