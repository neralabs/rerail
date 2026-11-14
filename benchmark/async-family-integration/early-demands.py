#!/usr/bin/env python3
"""Compare the merged engine, accepted late spike, and production early candidate lowering."""
import argparse,json,os,subprocess
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--before',type=Path,required=True);p.add_argument('--after',type=Path,required=True)
p.add_argument('--legacy-rules',type=Path,required=True);p.add_argument('--corpus',type=Path,required=True)
p.add_argument('--out',type=Path,required=True);p.add_argument('--mode',choices=['profiles','timings','edges'],required=True)
p.add_argument('--fixtures',type=Path);p.add_argument('--variants',default='merged,late,early')
a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True);records=[]
def run(name,variant,profile=False,corpus=None):
    e={k:v for k,v in os.environ.items() if not k.startswith('RERAIL_')}
    e.update(RUST_MIN_STACK='67108864',RERAIL_CORPUS_MEMO='off',RERAIL_CROSS_MEMO='off',
        RERAIL_TAPE_CACHE=str((a.out/(variant+'-cache.sqlite')).resolve()),
        RERAIL_SHARED_CORPUS=str((corpus or a.corpus).resolve()),RERAIL_RR_OUT=str((a.out/name).resolve()),RERAIL_RR_REPORT=str((a.out/(name+'.json')).resolve()))
    if variant!='early':
        e.update(RERAIL_RR_LIB=str((a.legacy_rules/'lib').resolve()),RERAIL_RR_RULE_DIR=str((a.legacy_rules/'packs/python').resolve()))
    if variant=='late':e.update(RERAIL_STABLE_SETUP_SPIKE='1',RERAIL_CANDIDATE_SPIKE='typed_ranges',RERAIL_DEMAND_FRAME_GUARD='1')
    if profile:e['RERAIL_PROFILE']=str((a.out/name/'profile').resolve())
    binary=a.after if variant=='early' else a.before
    cmd=['nice','-n','19',str(binary.resolve()),'complete_async_family_authored_rr_corpus_profile','--ignored','--nocapture','--test-threads=1']
    with (a.out/(name+'.log')).open('w') as log:r=subprocess.run(cmd,env=e,stdout=log,stderr=subprocess.STDOUT,timeout=600)
    record=dict(name=name,variant=variant,command=cmd,exitCode=r.returncode,environment={k:v for k,v in e.items() if k.startswith(('RERAIL_','RUST_MIN_STACK'))})
    if r.returncode==0:
        d=json.loads((a.out/(name+'.json')).read_text());record.update(cpu=d['cpuNs']/1e9,wall=d['wallNs']/1e9,files=d['report']['files'],keys=d['keys'],phases=d['report']['phases'])
    records.append(record);(a.out/(a.mode+'-processes.json')).write_text(json.dumps(records,indent=2)+'\n')
    print(name,r.returncode,record.get('cpu'),flush=True)
    if r.returncode:raise SystemExit(r.returncode)
    return record
variants=a.variants.split(',')
if 'early' in variants:
    listing=subprocess.check_output(['nice','-n','19',str(a.after.resolve()),'candidate_contract_lowers_before_projection_and_roundtrips','--list'],text=True)
    assert 'candidate_contract_lowers_before_projection_and_roundtrips: test' in listing,'after binary does not contain early demand lowering tests'
if a.mode=='profiles':
    for v in variants:run('warmup-'+v,v);run(v+'-profile',v,True)
elif a.mode=='timings':
    for v in variants:run('warmup-'+v,v)
    for i in range(3):
        samples=[run(f'{i}-{v}',v) for v in (variants if i%2==0 else variants[::-1])]
        assert all(r['keys']==samples[0]['keys'] and r['files']==samples[0]['files'] for r in samples)
else:
    if not a.fixtures:p.error('--fixtures required')
    for fixture in sorted(a.fixtures.iterdir()):
        samples=[run(f'{fixture.name}-{v}',v,True,fixture) for v in variants]
        assert all(r['keys']==samples[0]['keys'] for r in samples)
