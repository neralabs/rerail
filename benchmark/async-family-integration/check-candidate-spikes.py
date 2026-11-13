#!/usr/bin/env python3
"""Check corpus-specific spike gates against independent process reports."""
import argparse,json,statistics
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--profiles',type=Path,required=True)
p.add_argument('--timings',type=Path,required=True)
p.add_argument('--edges',type=Path,required=True)
p.add_argument('--positive',type=Path,required=True)
p.add_argument('--out',type=Path,required=True)
a=p.parse_args()
read=lambda path:json.loads(path.read_text())
baseline=read(a.profiles/'baseline-profile.json')
winner=read(a.profiles/'stable-frame_guard-profile.json')
def metric(d):
    decisions=d['profile']['decisions']
    return dict(files=d['report']['files'],findings=d['keyCount'],
        executions=sum(x[4] for x in d['report']['phases']),
        vm=sum(x['vmSteps'] for x in d['profile']['statements']),
        reprepares=(sum(int(x['text'])*x['count'] for x in decisions if x['topic']=='composed automatic reprepare') if any(x['topic']=='composed automatic reprepare' for x in decisions) else None),
        candidates=(sum(json.loads(x['text'])['emitted']*x['count'] for x in decisions if x['topic']=='candidate spike') if any(x['topic']=='candidate spike' for x in decisions) else None))
b,w=metric(baseline),metric(winner)
records=read(a.timings/'timings-processes.json')
medians={}
for variant in sorted({r['variant'] for r in records}):
    rows=[r for r in records if r['variant']==variant and not r['name'].startswith('warmup')]
    assert len(rows)==3 and all(r['exitCode']==0 and r['files']==5725 and not r['keys'] for r in rows)
    local=[sum(x[1]+x[2] for x in r['phases'] if x[0]=='local') for r in rows]
    medians[variant]={'cpu':statistics.median(r['cpu'] for r in rows),'wall':statistics.median(r['wall'] for r in rows),'localCpu':statistics.median(local),'localSamples':local}
edges=read(a.edges/'edges-processes.json')
edge_controls=[r for r in edges if r['variant']=='baseline']
assert len(edge_controls)==4
for control in edge_controls:
    prefix=control['name'].removesuffix('-baseline')
    assert all(r['exitCode']==0 and r['keys']==control['keys'] for r in edges if r['name'].startswith(prefix+'-'))
positive=read(a.positive)
gates={
    'baselineFailsNewVmAndReprepareCeilings':b['vm']>800_000 and b['reprepares']>0,
    'fullCorpusParity':w['files']==5725 and winner['keys']==baseline['keys']==[],
    'executionBudget':w['executions']<=100,
    'vmCeiling':w['vm']<=800_000,
    'noAutomaticReprepare':w['reprepares']==0,
    'narrowCandidates':w['candidates']==10,
    'edgeFindings':sum(len(r['keys']) for r in edge_controls)==178,
    'positiveFindings':positive['keyCount']==120 and positive['keys']==positive['expectedKeys'] and not positive['missingKeys'] and not positive['unexpectedKeys'],
    'positiveVmCeiling':metric(positive)['vm']<=80_372,
    'positiveExecutionBudget':metric(positive)['executions']<=100,
    'localCpuImprovement':medians['stable-frame_guard']['localCpu']<=0.75*medians['baseline']['localCpu'],
    'wholeCpuNoRegression':medians['stable-frame_guard']['cpu']<=1.10*medians['baseline']['cpu'],
}
report={'baseline':b,'winner':w,'medians':medians,'gates':gates,'allPassed':all(gates.values())}
a.out.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
assert report['allPassed'], 'spike acceptance failed; report preserved'
