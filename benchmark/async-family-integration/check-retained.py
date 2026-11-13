#!/usr/bin/env python3
"""Fail if the shared native candidate relation crosses the SQL boundary."""
import argparse
import json
from pathlib import Path
import re

p = argparse.ArgumentParser()
p.add_argument('report', type=Path)
p.add_argument('--output', required=True, type=Path)
a = p.parse_args()
d = json.loads(a.report.read_text())
statements = d['profile']['statements']
offenders = [s for s in statements if s['runs'] and
             re.search(r'\b(?:\w+_)?async_reference\b', s['sql'])]
reads = [r for r in d['profile']['decisions'] if r['topic'] == 'native demand read']
executions = sum(phase[4] for phase in d['report']['phases'])
result = dict(profiled=d['profiling'], files=d['report']['files'],
              executions=executions, execution_budget_100_met=executions <= 100,
              cpu_seconds=d['cpuNs'] / 1e9, wall_seconds=d['wallNs'] / 1e9,
              vm_steps=sum(s['vmSteps'] for s in statements),
              candidate_sql_vm=sum(s['vmSteps'] for s in offenders),
              candidate_sql=offenders, demand_reads=reads)
candidate_reads = [r for r in reads if re.match(r'(?:\w+_)?async_reference:', r['text'])]
loads = sum(r['count'] for r in candidate_reads if ': retained ' in r['text'])
result['candidate_snapshot_loads'] = loads
result['candidate_snapshot_cache_hits'] = sum(r['count'] for r in candidate_reads if ': cache ' in r['text'])
result['passed'] = (d['profiling'] and not offenders and
                    0 < loads <= d['report']['chunks'] and
                    all(': retained ' in r['text'] or ': cache ' in r['text'] for r in candidate_reads))
a.output.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({k: v for k, v in result.items() if k not in ('candidate_sql', 'demand_reads')}))
if not result['passed']:
    raise SystemExit('Retained candidate gate failed; see report')
