#!/usr/bin/env python3
"""Check early demand lowering against the merged control and accepted late experiment."""
import argparse,json,re,statistics
from pathlib import Path
p=argparse.ArgumentParser()
for n in ['control','early','timings','edges','positive','out']:p.add_argument('--'+n,type=Path,required=True)
a=p.parse_args();read=lambda f:json.loads(f.read_text())
def metrics(d):
    dec=d['profile']['decisions']
    projection=[(json.loads(x['text']),x['count']) for x in dec if x['topic']=='projection demand']
    candidates=[(json.loads(x['text']),x['count']) for x in dec if x['topic'] in ('Python candidate work','candidate spike')]
    return dict(files=d['report']['files'],findings=d['keyCount'],cpu=d['cpuNs']/1e9,wall=d['wallNs']/1e9,
        executions=sum(x[4] for x in d['report']['phases']),vm=sum(x['vmSteps'] for x in d['profile']['statements']),
        reprepares=sum(int(x['text'])*x['count'] for x in dec if x['topic']=='composed automatic reprepare'),
        reprepareSamples=sum(x['count'] for x in dec if x['topic']=='composed automatic reprepare'),
        projection={key:sum(x[key]*n for x,n in projection) for key in ('scalarNodes','nodes','strings')},
        candidateBatches=sum(n for _,n in candidates),candidateRows=sum(x['emitted']*n for x,n in candidates),
        semanticCalls=sum(x['count'] for x in dec if x['topic']=='SQL semantic batch' and not x['text'].startswith('candidates:')),
        semanticDemands=sum(int(re.search(r': (\d+) demanded',x['text'])[1])*x['count'] for x in dec if x['topic']=='SQL semantic batch' and not x['text'].startswith('candidates:')),
        spans={x['name']:{'cpu':x['cpuSeconds'],'ownCpu':x['ownCpuSeconds'],'allocatedBytes':x['allocatedBytes']} for x in d['profile']['spans']})
reports={v:read((a.early if v=='early' else a.control)/(v+'-profile.json')) for v in ['merged','late','early']}
m={v:metrics(d) for v,d in reports.items()}
records=read(a.timings/'timings-processes.json');medians={}
for v in ['merged','late','early']:
    rows=[r for r in records if r['variant']==v and not r['name'].startswith('warmup')]
    assert len(rows)==3 and all(r['exitCode']==0 and r['files']==5725 and not r['keys'] for r in rows)
    medians[v]={k:statistics.median(r[k] for r in rows) for k in ['cpu','wall']}
    medians[v]['localCpu']=statistics.median(sum(x[1]+x[2] for x in r['phases'] if x[0]=='local') for r in rows)
    medians[v]['workerBatchCpu']=statistics.median(sum(x[1]+x[2] for x in r['phases'] if x[0]=='batch (worker)') for r in rows)
edges=read(a.edges/'edges-processes.json');controls=[r for r in edges if r['variant']=='merged']
assert len(controls)==4
for c in controls:
    prefix=c['name'].removesuffix('-merged')
    assert all(r['exitCode']==0 and r['keys']==c['keys'] for r in edges if r['name'].startswith(prefix+'-'))
positive=read(a.positive);w=m['early']
gates={
 'fullCorpusParity':all(d['report']['files']==5725 and d['keys']==[] for d in reports.values()),
 'executionBudget':w['executions']<=100,
 'vmCeiling':w['vm']<=800_000,
 'noAutomaticReprepare':w['reprepares']==0 and w['reprepareSamples']==reports['early']['report']['chunks']+1,
 'oneCandidateProducerPerBatch':w['candidateBatches']==reports['early']['report']['chunks'],
 'narrowCandidates':w['candidateRows']==m['late']['candidateRows']==10,
 'noScalarNodeMaterialization':w['projection']['nodes']>0 and w['projection']['scalarNodes']==0,
 'smallVocabulary':w['projection']['strings']<=50_000,
 'noSemanticAmplification':w['semanticDemands']<=m['late']['semanticDemands'],
 'edgeFindings':sum(len(r['keys']) for r in controls)==178,
 'positiveFindings':positive['keyCount']==120 and positive['keys']==positive['expectedKeys'] and not positive['missingKeys'] and not positive['unexpectedKeys'],
 'positiveExecutionBudget':metrics(positive)['executions']<=100,
 'wholeCpuImprovementOverLate':medians['early']['cpu']<medians['late']['cpu']*.9,
 'workerBatchCpuImprovementOverLate':medians['early']['workerBatchCpu']<medians['late']['workerBatchCpu']*.5,
}
report={'profiles':m,'medians':medians,'gates':gates,'allPassed':all(gates.values())}
a.out.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'medians':medians,'gates':gates},indent=2))
assert report['allPassed'],'early candidate acceptance failed; report preserved'
