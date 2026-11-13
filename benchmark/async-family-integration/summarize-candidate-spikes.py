#!/usr/bin/env python3
"""Summarize actual process profiles, keeping replay and index-matrix timings separate."""
import argparse,json,re,statistics
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('directory',type=Path);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
out={}
for path in sorted(a.directory.rglob('*.json')):
    d=json.loads(path.read_text())
    if not isinstance(d,dict) or 'profile' not in d:continue
    phases=d['report']['phases'];profile=d['profile'];counts={}
    for decision in profile['decisions']:
        topic=decision['topic'];text=decision['text'];count=decision['count']
        if topic in ['candidate spike','lookup spike']:
            values=json.loads(text);total=counts.setdefault(topic,{})
            for k,v in values.items():total[k]=total.get(k,0)+v*count
        elif topic=='composed automatic reprepare':
            counts[topic]=counts.get(topic,0)+int(text)*count
        elif topic=='python demand':
            kind,text=text.split(': ',1);total=counts.setdefault('semantic:'+kind,{})
            for k,v in re.findall(r'(\w+)=(\d+)',text):total[k]=total.get(k,0)+int(v)*count
        elif topic=='composed input work':
            name,text=text.split(': ',1);total=counts.setdefault('input:'+name,[0]*4)
            for i,v in enumerate(map(int,re.findall(r'\d+',text))):total[i]+=v*count
    spans=profile['spans']
    row=dict(cpu=d['cpuNs']/1e9,wall=d['wallNs']/1e9,localCpu=sum(p[1]+p[2] for p in phases if p[0]=='local'),files=d['report']['files'],findings=d['keyCount'],keys=d['keys'],chunks=d['report']['chunks'],executions=sum(p[4] for p in phases),vm=sum(s['vmSteps'] for s in profile['statements']),counts=counts,
        lookupCpu=sum(s['ownCpuSeconds'] for s in spans if s.get('name',s.get('path','')).endswith('SQL IN index lookup')),
        decodeCpu=sum(s['ownCpuSeconds'] for s in spans if s.get('name',s.get('path','')).endswith('SQL IN key decode')),
        candidateCpu=sum(s['ownCpuSeconds'] for s in spans if s.get('name',s.get('path','')).endswith('native candidate spike')))
    out[str(path.relative_to(a.directory))]=row
    print(path.name, f'{row["cpu"]:.3f}s job, {row["localCpu"]:.3f}s local, {row["candidateCpu"]:.3f}s producer, {row["vm"]:,} VM, {row["executions"]} SQL, {row["findings"]} findings')
a.output.write_text(json.dumps(out,indent=2)+'\n')
