#!/usr/bin/env python3
"""Pin SQL work on the full corpus and a nonempty, four-rule positive workload."""
import argparse
import gzip
import json
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--corpus', type=Path, required=True)
p.add_argument('--positive', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()

def read(path):
    with (gzip.open if path.suffix == '.gz' else open)(path, 'rt') as f:
        return json.load(f)

full, positive = read(a.corpus), read(a.positive)
results = {}
for name, d, ceiling in [('corpus', full, 3_600_000), ('positive', positive, 150_000)]:
    results[name] = dict(profiled=d['profiling'], files=d['report']['files'], findings=d['keyCount'],
                         executions=sum(r[4] for r in d['report']['phases']),
                         vm=sum(s['vmSteps'] for s in d['profile']['statements']), ceiling=ceiling)
gates = dict(full_workload=full['report']['files']==5725 and full['report']['nodes']==19007630,
             full_parity=full['keyCount']==0 and not full['keys'],
             positive_workload=positive['report']['files']==24 and positive['keyCount']==120,
             positive_parity=not positive['missingKeys'] and not positive['unexpectedKeys'])
for name, r in results.items():
    gates[name+'_profiled'] = r['profiled']
    gates[name+'_statement_budget'] = 0 < r['executions'] <= 100
    gates[name+'_vm_ceiling'] = 0 < r['vm'] <= r['ceiling']
result = dict(passed=all(gates.values()), gates=gates, measurements=results)
a.output.write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(result, indent=2))
raise SystemExit(not result['passed'])
