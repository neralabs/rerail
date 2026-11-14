#!/usr/bin/env python3
"""Enforce the complete-job SQL budget and inspect the composed evaluation path."""
import argparse
import gzip
import json
from pathlib import Path
import re

p = argparse.ArgumentParser()
p.add_argument('report', type=Path)
p.add_argument('--output', required=True, type=Path)
a = p.parse_args()
read = gzip.open if a.report.suffix == '.gz' else open
with read(a.report, 'rt') as f:
    d = json.load(f)
statements = d['profile']['statements']
executions = sum(phase[4] for phase in d['report']['phases'])
evaluation = [s for s in statements if 'composed relational query' in s['group']]
wave = [s for s in statements if re.search(r'\bwave_(?:step|pending|delta)\b', s['sql'])]
publication = [s for s in statements if re.match(
    r'(?:CREATE\s+(?:TEMP\s+)?TABLE|INSERT(?:\s+OR\s+IGNORE)?\s+INTO|DROP\s+TABLE(?:\s+IF\s+EXISTS)?)\s+(?:temp\.)?"?(?:[pq]\d+_|async_)',
    s['sql'], re.I)]
count = sum(s['runs'] for s in evaluation)
candidate_exports = [s for s in evaluation if re.search(
    r'\bFROM\s+"?async_reference"?(?:\s+UNION ALL|\s*$)',
    s['sql'].split('SELECT 0 AS __output', 1)[-1], re.I)]
failures = []
if not d['profiling']:
    failures.append('A profiled production run is required.')
if executions > 100:
    failures.append(f'Whole-job executions exceed 100: {executions}.')
if not count or count > d['report']['chunks'] + 2:
    failures.append(f'Expected one composed evaluation per bounded batch, plus final composition: {count}.')
if wave:
    failures.append('The acyclic job still executes SQL wave control statements.')
if publication:
    failures.append('The job still publishes intermediate rule relations to SQL tables.')
if candidate_exports:
    failures.append('Dead native dependencies still export the shared candidate relation.')
result = dict(passed=not failures, failures=failures, executions=executions,
              relational_executions=count, chunks=d['report']['chunks'],
              vm_steps=sum(s['vmSteps'] for s in statements),
              evaluation_vm_steps=sum(s['vmSteps'] for s in evaluation),
              sql_bytes=sorted(set(len(s['sql'].encode()) for s in evaluation)),
              publication=publication, wave=wave)
a.output.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({k:v for k,v in result.items() if k not in ('publication','wave')}, indent=2))
if failures:
    raise SystemExit(1)
