#!/usr/bin/env python3
"""Separate-process candidate and index experiments, with explicit cache and scope boundaries."""
import argparse,json,os,subprocess
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--binary',type=Path,required=True)
p.add_argument('--corpus',type=Path,required=True)
p.add_argument('--out',type=Path,required=True)
p.add_argument('--mode',choices=['profiles','timings','edges','lookup','matrix'],required=True)
p.add_argument('--fixtures',type=Path)
p.add_argument('--variants',default='baseline,structural,context_first,screen_first,context_ranges')
a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True)
env=os.environ.copy()
for k in list(env):
    if k.startswith('RERAIL_'):env.pop(k)
env.update(RUST_MIN_STACK='67108864',RERAIL_CORPUS_MEMO='off',RERAIL_CROSS_MEMO='off',RERAIL_TAPE_CACHE=str((a.out/'tape-cache.sqlite').resolve()))
records=[]
def run(name,variant='baseline',profile=False,corpus=None,test=None,extra=None):
    e=env|dict(RERAIL_SHARED_CORPUS=str((corpus or a.corpus).resolve()),RERAIL_RR_OUT=str((a.out/name).resolve()),RERAIL_RR_REPORT=str((a.out/(name+'.json')).resolve()))
    physical=variant.removeprefix('stable-')
    if variant.startswith('stable-'):e['RERAIL_STABLE_SETUP_SPIKE']='1'
    if physical in ['structural','context_first','screen_first','context_ranges','typed_ranges','typed_guard','frame_guard']:e['RERAIL_CANDIDATE_SPIKE']='typed_ranges' if physical in ['typed_guard','frame_guard'] else physical
    if physical=='typed_guard':e['RERAIL_EMPTY_DEMAND_GUARD']='1'
    if physical=='frame_guard':e['RERAIL_DEMAND_FRAME_GUARD']='1'
    elif variant.startswith('lookup-'):
        e['RERAIL_LOOKUP_SQL_IN']='1'
        if variant!='lookup-original':e['RERAIL_LOOKUP_SPIKE']=variant.removeprefix('lookup-')
        if profile:e['RERAIL_LOOKUP_TRACE']='1'
    if profile:e['RERAIL_PROFILE']=str((a.out/name/'profile').resolve())
    e.update(extra or {})
    cmd=['nice','-n','19',str(a.binary.resolve()),test or 'complete_async_family_authored_rr_corpus_profile','--ignored','--nocapture','--test-threads=1']
    with (a.out/(name+'.log')).open('w') as log:
        try:r=subprocess.run(cmd,env=e,stdout=log,stderr=subprocess.STDOUT,timeout=600);code=r.returncode
        except subprocess.TimeoutExpired:code=124
    record=dict(name=name,variant=variant,command=cmd,exitCode=code,environment={k:v for k,v in e.items() if k.startswith('RERAIL_') or k=='RUST_MIN_STACK'})
    if code==0 and test is None:
        d=json.loads((a.out/(name+'.json')).read_text());record.update(cpu=d['cpuNs']/1e9,wall=d['wallNs']/1e9,files=d['report']['files'],keys=d['keys'],phases=d['report']['phases'],prepared=d['report']['prepared'])
    records.append(record);(a.out/(a.mode+'-processes.json')).write_text(json.dumps(records,indent=2)+'\n');print(name,code,record.get('cpu'),flush=True)
    if code:raise SystemExit(code)
    return record
variants=a.variants.split(',')
if a.mode=='profiles':
    run('warmup')
    for variant in variants:run(variant+'-profile',variant,True)
elif a.mode=='timings':
    for variant in variants:run('warmup-'+variant,variant)
    for i in range(3):
        for variant in (variants if i%2==0 else variants[::-1]):run(f'{i}-{variant}',variant)
    samples=[r for r in records if not r['name'].startswith('warmup')]
    assert all(r['keys']==samples[0]['keys'] and r['files']==samples[0]['files'] for r in samples)
elif a.mode=='edges':
    if not a.fixtures:p.error('--fixtures required')
    for fixture in sorted(a.fixtures.iterdir()):
        base=run(f'{fixture.name}-baseline',corpus=fixture)
        for variant in variants:
            if variant=='baseline':continue
            got=run(f'{fixture.name}-{variant}',variant,True,fixture)
            assert got['keys']==base['keys'],f'{fixture.name}: {variant} changes findings'
elif a.mode=='lookup':
    run('warmup')
    for variant in ['baseline','lookup-original','lookup-point','lookup-merge','lookup-gallop','lookup-adaptive']:run(variant+'-profile',variant,True)
else:
    run('lookup-matrix',test='lookup_shape_matrix',extra={'RERAIL_LOOKUP_REPORT':str((a.out/'lookup-matrix.json').resolve())})
