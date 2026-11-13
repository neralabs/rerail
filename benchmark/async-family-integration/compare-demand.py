#!/usr/bin/env python3
"""Compare complete authored jobs and inspect demand counters without rewriting evidence."""
import argparse
import json
from pathlib import Path
import re

parser = argparse.ArgumentParser()
parser.add_argument('--baseline', required=True, type=Path)
parser.add_argument('--after', required=True, type=Path)
parser.add_argument('--legacy', type=Path)
parser.add_argument('--output', required=True, type=Path)
a = parser.parse_args()


def summarize(path):
    data = json.loads(path.read_text())
    profile = data['profile']
    demand = {}
    for decision in profile['decisions']:
        if decision['topic'] != 'python demand':
            continue
        kind, counters = decision['text'].split(': ', 1)
        totals = demand.setdefault(kind, {})
        for name, value in re.findall(r'(\w+)=(\d+)', counters):
            totals[name] = totals.get(name, 0) + int(value) * decision['count']
    statement_types = {}
    for statement in profile['statements']:
        kind = statement['sql'].lstrip().split(maxsplit=1)[0].upper()
        statement_types[kind] = statement_types.get(kind, 0) + statement['runs']
    aggregate_plans = []
    trace = Path(profile['directory']) / 'native.tsv' if profile['directory'] else None
    if trace and trace.exists():
        for line in trace.read_text().splitlines()[1:]:
            span, work, steps, plan = line.split('\t', 3)
            if re.match(r'seed \S*rr_agg\d', plan):
                aggregate_plans.append(dict(span=span, work=int(work), plan=plan,
                    max_step_outputs=max(int(step.split(':')[1]) for step in steps.split(','))))
    return dict(
        source=str(path), cpu_seconds=data['cpuNs'] / 1e9,
        wall_seconds=data['wallNs'] / 1e9,
        files=data['report']['files'], nodes=data['report']['nodes'],
        chunks=data['report']['chunks'], prepared=data['report']['prepared'],
        executions=sum(phase[4] for phase in data['report']['phases']),
        vm_steps=sum(s['vmSteps'] for s in profile['statements']),
        findings=data['keyCount'], demands=demand, statement_types=statement_types,
        aggregate_plans=sorted(aggregate_plans, key=lambda r: r['max_step_outputs'], reverse=True)[:12],
        phases=data['report']['phases'],
        top_own_cpu=sorted(profile['spans'], key=lambda s: s['ownCpuSeconds'], reverse=True)[:12],
    )


result = {'schema': 'rerail-demand-repair-v1', 'runs': {}, 'edges_vs_ruff': []}
for variant, directory in [('baseline', a.baseline), ('demand', a.after), ('legacy', a.legacy)]:
    if directory is None:
        continue
    for mode in ('rr', 'rr-profile'):
        path = directory / (mode + '.json')
        if path.exists():
            result['runs'][f'{variant}-{mode}'] = summarize(path)

ruff = json.loads((a.baseline / 'ruff-edges.json').read_text())
for path in sorted(a.after.glob('edge-*.json')):
    if path.name == 'edge-parity.json':
        continue
    fixture = path.stem.removeprefix('edge-')
    data = json.loads(path.read_text())
    expected = {(r['code'], Path(r['filename']).name, r['location']['row'])
                for r in ruff if Path(r['filename']).parent.name == fixture}
    actual = set(map(tuple, data['keys']))
    result['edges_vs_ruff'].append(dict(fixture=fixture, expected=len(expected), actual=len(actual),
                                      missing=sorted(expected - actual), unexpected=sorted(actual - expected)))
for mode in ('rr', 'rr-profile'):
    before = result['runs'].get(f'baseline-{mode}')
    after = result['runs'].get(f'demand-{mode}')
    if before and after:
        result.setdefault('ratios', {})[mode] = {
            key: before[key] / after[key] if after[key] else None
            for key in ('cpu_seconds', 'wall_seconds', 'executions', 'vm_steps')}
fixtures = {p.name for p in (a.baseline / 'edge-fixtures').iterdir() if p.is_dir()}
result['gates'] = {
    'all_ruff_edge_fixtures_measured': {r['fixture'] for r in result['edges_vs_ruff']} == fixtures,
    'ruff_edges_match': all(not r['missing'] and not r['unexpected'] for r in result['edges_vs_ruff']),
}
if a.legacy:
    for fixture in sorted(fixtures):
        name = f'edge-{fixture}.json'
        old, new = a.baseline / name, a.legacy / name
        result['gates']['legacy_' + fixture + '_unchanged'] = (
            new.exists() and json.loads(old.read_text())['keys'] == json.loads(new.read_text())['keys'])
for mode in ('rr', 'rr-profile'):
    before_path, after_path = a.baseline / (mode + '.json'), a.after / (mode + '.json')
    if before_path.exists() and after_path.exists():
        before, after = json.loads(before_path.read_text()), json.loads(after_path.read_text())
        result['gates'][mode + '_full_workload_matches'] = all(
            before['report'][key] == after['report'][key] for key in ('files', 'nodes'))
        result['gates'][mode + '_findings_match'] = before['keys'] == after['keys']
a.output.write_text(json.dumps(result, indent=2) + '\n')
print(a.output)
if not all(result['gates'].values()):
    raise SystemExit('Demand repair validation failed; see gates in comparison JSON')
