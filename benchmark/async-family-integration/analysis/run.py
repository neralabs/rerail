import os,json,subprocess,pathlib
root=pathlib.Path('/Users/jeremy/orca/artifacts/async-family-comparison/analysis-mode')
binary=pathlib.Path('/Users/jeremy/orca/artifacts/async-family-comparison/target/profiling/deps/rerail_engine-9943fa461d0e6845')
listing=subprocess.check_output(['nice','-n','19',str(binary),'production_adapter_emits_ranked_findings_and_json','--list'],text=True)
assert 'production_adapter_emits_ranked_findings_and_json: test' in listing,'stale executable'
base={k:v for k,v in os.environ.items() if not k.startswith('RERAIL_')};base['RUST_MIN_STACK']='67108864'
with (root/'integrated-tests.log').open('w') as f:
 subprocess.run(['nice','-n','19',str(binary),'logic::profile::analysis','--nocapture','--test-threads=1'],env=base,stdout=f,stderr=subprocess.STDOUT,check=True)
records=[]
for variant in ['late','current']:
 out=root/variant;e=base.copy();e.update(RERAIL_CORPUS_MEMO='off',RERAIL_CROSS_MEMO='off',RERAIL_SHARED_CORPUS='/Users/jeremy/orca/artifacts/async-family-comparison/corpus-fce4dc72',RERAIL_RR_OUT=str(out),RERAIL_RR_REPORT=str(root/(variant+'.json')),RERAIL_PROFILE=str(out/'profile'))
 if variant=='late':
  e.update(RERAIL_TAPE_CACHE='/Users/jeremy/orca/artifacts/async-family-comparison/early-demands/control/late-cache.sqlite',RERAIL_RR_LIB='/Users/jeremy/orca/artifacts/async-family-comparison/early-demands/baseline-rules/lib',RERAIL_RR_RULE_DIR='/Users/jeremy/orca/artifacts/async-family-comparison/early-demands/baseline-rules/packs/python',RERAIL_CANDIDATE_SPIKE='typed_ranges')
 else:e['RERAIL_TAPE_CACHE']='/Users/jeremy/orca/artifacts/async-family-comparison/early-demands/production/early-cache.sqlite'
 cmd=['nice','-n','19',str(binary),'complete_async_family_authored_rr_corpus_profile','--ignored','--nocapture','--test-threads=1']
 with (root/(variant+'.log')).open('w') as f:subprocess.run(cmd,env=e,stdout=f,stderr=subprocess.STDOUT,check=True,timeout=600)
 d=json.loads((root/(variant+'.json')).read_text());a=json.loads((out/'profile/analysis.json').read_text())
 assert d['report']['files']==5725 and d['keys']==[]
 codes={f['code'] for f in a['findings']}
 if variant=='late':assert 'LATE_PROJECTION' in codes
 else:assert 'LATE_PROJECTION' not in codes and 'UNREQUESTED_SCALARS' not in codes and 'REPREPARE' not in codes and 'CACHED_TAPE_REBUILD' in codes and 'IDENTITY_PROJECTION' in codes
 records.append(dict(variant=variant,command=cmd,environment={k:v for k,v in e.items() if k.startswith(('RERAIL_','RUST_MIN_STACK'))},cpu=d['cpuNs']/1e9,wall=d['wallNs']/1e9,findings=a['findings']))
 (root/'runs.json').write_text(json.dumps(records,indent=2)+'\n')
 print(variant,records[-1]['cpu'],codes,flush=True)
e=base.copy();e.update(RERAIL_RR_POSITIVE_REPORT=str(root/'positive.json'),RERAIL_PROFILE=str(root/'positive-profile'))
with (root/'positive.log').open('w') as f:subprocess.run(['nice','-n','19',str(binary),'authored_rr_batched_positive_documents_preserve_all_four_labels','--nocapture','--test-threads=1'],env=e,stdout=f,stderr=subprocess.STDOUT,check=True)
print('positive findings',json.loads((root/'positive.json').read_text())['keyCount'],flush=True)
