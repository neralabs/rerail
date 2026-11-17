#!/usr/bin/env python3
"""Summarize measured boundaries without equating the different runners."""
import json
from pathlib import Path
from statistics import median
import sys
root = Path(sys.argv[1])
r = json.loads((root / 'reference-full.json').read_text())
runs = r['runs']
ms = lambda values: round(median(values) / 1e6, 3)
out = {'files': r['files'], 'source_bytes': r['source_bytes'],
       'reference': {
           'indexed_native_match_cpu_ms': ms([x['native_match_cpu_ns'] for x in runs]),
           'current_native_execution_cpu_ms': ms([x['current_native_execution_cpu_ns'] for x in runs]),
           'current_native_selection_cpu_ms': ms([x['current_native_selection_cpu_ns'] for x in runs]),
           'independent_cpu_ms': ms([x['reference']['execution_ns'] for x in runs]),
           'generated_cpu_ms': ms([x['generated']['execution_ns'] for x in runs]),
           'independent_wall_ms': ms([x['reference']['wall_ns'] for x in runs]),
           'generated_wall_ms': ms([x['generated']['wall_ns'] for x in runs]),
           'parse_tape_cpu_ms': r['parse_tape_cpu_ns']/1e6,
           'indexed_native_index_cpu_ms': r['index_construction_cpu_ns']/1e6,
           'indexed_native_first_selection_cpu_ms': runs[0]['native_selection_cpu_ns']/1e6,
           'indexed_native_repeat_selection_cpu_ms': [x['native_selection_cpu_ns']/1e6 for x in runs[1:]],
           'semantic_requests': [x['generated']['semantic_requests'] for x in runs],
           'generated_vm': [x['generated']['vm_steps'] for x in runs],
           'findings': [x['findings'] for x in runs]},
       'json_admission': r['generic_compiler_admission']}
for name in ['rr', 'rr-profile', 'rr-positive']:
    path = root / (name + '.json')
    if not path.exists():
        continue
    p = json.loads(path.read_text())
    out[name] = {'cpu_ms': p['cpuNs']/1e6, 'wall_ms': p['wallNs']/1e6,
                 'files': p['report']['files'], 'nodes': p['report']['nodes'],
                 'findings': p['keyCount'], 'profiling': p['profiling'],
                 'prepared': p['report']['prepared'], 'memo': p['report']['memo'],
                 'phases': p['report']['phases'], 'chunks': p['report']['chunks'],
                 'sql_executions': sum(x[4] for x in p['report']['phases']),
                 'execution_budget_100_pass': sum(x[4] for x in p['report']['phases']) <= 100}
    sql = p['profile']['statements']
    if sql:
        out[name]['all_traced_sql'] = {k: sum(x[k] for x in sql) for k in ['runs', 'vmSteps', 'fullScanSteps', 'sorts', 'autoindexRows']}
    if 'missingKeys' in p:
        out[name]['missing'] = p['missingKeys']
        out[name]['unexpected'] = p['unexpectedKeys']
for name in ['producers', 'edge-parity', 'ruff-edge-parity']:
    path = root / (name + '.json')
    if path.exists():
        out[name] = json.loads(path.read_text())
print(json.dumps(out, indent=2))
