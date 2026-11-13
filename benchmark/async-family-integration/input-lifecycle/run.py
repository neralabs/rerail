import os,json,subprocess,pathlib,hashlib,statistics
root=pathlib.Path(__file__).resolve().parent
binary=pathlib.Path('/Users/jeremy/orca/artifacts/async-family-comparison/target/profiling/deps/rerail_engine-9943fa461d0e6845')
listing=subprocess.check_output(['nice','-n','19',str(binary),'interval_lines_equal','--list'],text=True)
assert 'interval_lines_equal_binary_search_at_every_byte_in_both_directions: test' in listing,'stale executable'
base={k:v for k,v in os.environ.items() if not k.startswith('RERAIL_')}
base.update(RUST_MIN_STACK='67108864',RERAIL_THREADS='4',RERAIL_CORPUS_MEMO='off',RERAIL_CROSS_MEMO='off',RERAIL_TAPE_CACHE='/Users/jeremy/orca/artifacts/async-family-comparison/early-demands/production/early-cache.sqlite')
records=[]
variants={'baseline':'','lines':'lines','postings':'postings','reserve':'reserve','combined':'lines,postings,reserve'}
def invoke(name,env,test,ignored=False):
 cmd=['nice','-n','19',str(binary),test,'--nocapture','--test-threads=1']+(['--ignored'] if ignored else [])
 with (root/(name+'.log')).open('w') as f:subprocess.run(cmd,env=env,stdout=f,stderr=subprocess.STDOUT,check=True,timeout=600)
def corpus(variant,label,profile=False):
 name=variant+'-'+label; out=root/name; env=base.copy()
 env.update(RERAIL_INPUT_SPIKE=variants[variant],RERAIL_SHARED_CORPUS='/Users/jeremy/orca/artifacts/async-family-comparison/corpus-fce4dc72',RERAIL_RR_OUT=str(out),RERAIL_RR_REPORT=str(root/(name+'.json')))
 if profile:env['RERAIL_PROFILE']=str(out/'profile')
 invoke(name,env,'complete_async_family_authored_rr_corpus_profile',True)
 d=json.loads((root/(name+'.json')).read_text())
 assert d['report']['files']==5725 and d['keys']==[]
 r=dict(variant=variant,label=label,profile=profile,cpu=d['cpuNs']/1e9,wall=d['wallNs']/1e9,phases=d['report']['phases'],environment={k:v for k,v in env.items() if k.startswith(('RERAIL_','RUST_'))})
 records.append(r);(root/'runs.json').write_text(json.dumps(records,indent=2)+'\n')
 print(name,round(r['cpu'],4),round(r['wall'],4),flush=True)
for variant in ['baseline','combined']:
 env=base.copy();env['RERAIL_INPUT_SPIKE']=variants[variant]
 for test in ['facts::core::','facts::vtab::syntax::tests::','logic::profile::analysis','logic::plan::compose::tests::','authored_rr_batched_positive_documents_preserve_all_four_labels']:
  invoke(variant+'-'+test.replace(':','_'),env,test)
for variant in variants:corpus(variant,'profile',True)
for repetition in range(3):
 order=list(variants) if repetition%2==0 else list(reversed(variants))
 for variant in order:corpus(variant,'timed'+str(repetition))
summary={v:{key:statistics.median(r[key] for r in records if r['variant']==v and not r['profile']) for key in ['cpu','wall']} for v in variants}
(root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2),flush=True)
