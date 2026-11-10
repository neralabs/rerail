import pathlib
source=pathlib.Path(__file__).with_name('run.py').read_text()
exec(source[:source.index("checks=['facts::core::'")])
checks=[]
for rep in range(2):
 for arm in (['baseline','production'] if rep==0 else ['production','baseline']):
  name=arm+'-containment'+str(rep);m=invoke(arm,name,'facts::vtab::syntax::tests::within_holds_exactly_the_contained_pairs',base)
  m.update(arm=arm,rep=rep);checks.append(m);print(name,m['processCpu'],m['processWall'],flush=True)
(root/'containment.json').write_text(json.dumps(checks,indent=2)+'\n')
e=base.copy();e.update(RERAIL_TAPE_CACHE='off',RERAIL_SHARED_CORPUS='/Users/jeremy/orca/artifacts/async-family-comparison/corpus-fce4dc72',RERAIL_RR_OUT=str(root/'production-uncached'),RERAIL_RR_REPORT=str(root/'production-uncached.json'),RERAIL_PROFILE=str(root/'production-uncached/profile'))
m=invoke('production','production-uncached','complete_async_family_authored_rr_corpus_profile',e,True)
d=json.loads((root/'production-uncached.json').read_text());assert d['report']['files']==5725 and d['keys']==[]
spans=d['profile']['spans'];assert sum(s['calls'] for s in spans if s['name'].endswith('validated tape columns') and 'tree-sitter and facts' in s['name'])==5725
assert not any('tree-sitter and facts' in s['name'] and s['name'].endswith('tape restore') for s in spans)
m['environment']={k:v for k,v in e.items() if k.startswith(('RERAIL_','RUST_'))};(root/'uncached-process.json').write_text(json.dumps(m,indent=2)+'\n')
print('uncached',d['cpuNs']/1e9,d['wallNs']/1e9,flush=True)
