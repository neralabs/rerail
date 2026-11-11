import argparse,os,pathlib,subprocess,json,time,re
p=argparse.ArgumentParser();p.add_argument('--binary',required=True);p.add_argument('--out',required=True);p.add_argument('--mode',choices=['native','composed'],required=True);p.add_argument('--baseline',action='store_true');a=p.parse_args()
out=pathlib.Path(a.out);out.mkdir(parents=True,exist_ok=True)
env={k:v for k,v in os.environ.items() if not k.startswith('RERAIL_')};env.update(RUST_MIN_STACK='67108864',RERAIL_THREADS='4');env['RERAIL_COMPOSE']='1' if a.mode=='composed' else '0'
listing=subprocess.check_output(['nice','-n','19',a.binary,'corpus::session::tests','--list'],env=env,text=True)
tests=[l.split(': test')[0] for l in listing.splitlines() if ': test' in l and 'session_costs' not in l]
tests+=['a_cache_in_a_session_is_facts_and_a_version_bump_derives_it_again','a_cache_is_facts_and_a_version_bump_derives_it_again','cross_file_strata_are_kept_across_runs_in_the_tape_cache','a_relation_two_programs_share_is_kept_once_and_a_rule_edit_reaches_only_its_readers']
if a.baseline: tests=[t for t in tests if 'a_cache_in_a_session' in t or 'around_an_inlined' in t or 'strata_are_kept_across' in t]
records=[]
for i,t in enumerate(tests):
 log=out/(t.split('::')[-1]+'.log');start=time.monotonic()
 with log.open('w') as f:r=subprocess.run(['nice','-n','19',a.binary,t,'--nocapture','--test-threads=1'],env=env,stdout=f,stderr=subprocess.STDOUT)
 text=log.read_text(); counts=re.findall(r'test result: .*? (\d+) passed; (\d+) failed; (\d+) ignored',text)
 record=dict(test=t,mode=a.mode,exit=r.returncode,wall=time.monotonic()-start,counts=counts)
 records.append(record);(out/'validation.json').write_text(json.dumps(records,indent=2)+'\n');print(record,flush=True)

if not a.baseline and any(r["exit"] or not r["counts"] or sum(int(x) for x in r["counts"][0][:2])==0 for r in records): raise SystemExit(1)
