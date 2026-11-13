import os,json,subprocess,pathlib,resource,time,statistics
root=pathlib.Path(__file__).resolve().parent
bins={'baseline':root/'baseline-tests','production':pathlib.Path('/Users/jeremy/orca/artifacts/async-family-comparison/target/profiling/deps/rerail_engine-9943fa461d0e6845')}
base={k:v for k,v in os.environ.items() if not k.startswith('RERAIL_')}
base.update(RUST_MIN_STACK='67108864',RERAIL_THREADS='4',RERAIL_CORPUS_MEMO='off',RERAIL_CROSS_MEMO='off',RERAIL_TAPE_CACHE='/Users/jeremy/orca/artifacts/async-family-comparison/early-demands/production/early-cache.sqlite')
listing=subprocess.check_output(['nice','-n','19',str(bins['production']),'direct_tape_columns_equal_restored_columns_and_line_contracts','--list'],text=True)
assert 'direct_tape_columns_equal_restored_columns_and_line_contracts: test' in listing
records=[]
def invoke(arm,name,test,env,ignored=False):
 cmd=['nice','-n','19',str(bins[arm]),test,'--nocapture','--test-threads=1']+(['--ignored'] if ignored else [])
 before=resource.getrusage(resource.RUSAGE_CHILDREN);wall=time.perf_counter()
 with (root/(name+'.log')).open('w') as f:
  result=subprocess.run(cmd,env=env,stdout=f,stderr=subprocess.STDOUT,timeout=900)
 elapsed=time.perf_counter()-wall;after=resource.getrusage(resource.RUSAGE_CHILDREN)
 assert result.returncode==0, f'{name} failed: '+(root/(name+'.log')).read_text()[-2500:]
 return {'processCpu':after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime,'processWall':elapsed,'command':cmd}
checks=['facts::core::','tape::tests::','facts::vtab::syntax::','logic::profile::analysis','logic::plan::compose::tests::','authored_rr_batched_positive_documents_preserve_all_four_labels']
rr=['combined_programs_see_guests_by_their_own_policy','guests_that_do_not_parse_follow_the_policy','kind_sets_over_a_guest_are_its_grammars','string_annotations_are_guests_at_their_host_lines','match_all_captures_spans_over_a_nodes_text_and_over_captures','match_all_captures_spans_over_source_text_and_symbols_over_symbols','match_all_over_nodes_of_a_kind_and_over_captures_gives_spans','requested_source_and_line_text_remain_exact_without_copying_large_subtrees','source_span_producers_retain_rows_through_local_planning','python_split_lines_are_lossless_and_document_local','lf_lines_keep_other_separators_and_the_trailing_empty_line','parse_splice_and_unparse_make_value_documents','scope_takes_option_dirs_text_documents_and_report_dirs']
for t in checks+['logic::rr::tests::'+t for t in rr]:
 invoke('production','test-'+t.replace(':','_'),t,base)
 print('PASS',t,flush=True)
for t in ['logic::rr::tests::definition_ids_keep_other_definitions_rows_across_a_body_edit','authored_rr_batched_positive_documents_preserve_all_four_labels','projection_axes_and_token_spans_are_demanded_without_renumbering']:
 e=base.copy();e['RERAIL_NODE_IDS']='definition';invoke('production','definition-'+t.split(':')[-1],t,e);print('PASS definition',t,flush=True)
def corpus(arm,label,profile=False,corpus=None):
 name=arm+'-'+label;e=base.copy();e.update(RERAIL_SHARED_CORPUS=str(corpus or '/Users/jeremy/orca/artifacts/async-family-comparison/corpus-fce4dc72'),RERAIL_RR_OUT=str(root/name),RERAIL_RR_REPORT=str(root/(name+'.json')))
 if profile:e['RERAIL_PROFILE']=str(root/name/'profile')
 r=invoke(arm,name,'complete_async_family_authored_rr_corpus_profile',e,True)
 d=json.loads((root/(name+'.json')).read_text());r.update(arm=arm,label=label,profile=profile,cpu=d['cpuNs']/1e9,wall=d['wallNs']/1e9,flattenCpu=d['flattenCpuNs']/1e9,files=d['report']['files'],keys=d['keyCount'],sql=sum(p[4] for p in d['report']['phases']),phases=d['report']['phases'],environment={k:v for k,v in e.items() if k.startswith(('RERAIL_','RUST_'))})
 if corpus is None:assert d['report']['files']==5725 and d['keys']==[]
 else:
  old=json.loads((pathlib.Path('/Users/jeremy/orca/artifacts/async-family-comparison/retained-after')/('edge-'+pathlib.Path(corpus).name+'.json')).read_text());assert d['keys']==old['keys'],name+' edge parity'
 records.append(r);(root/'runs.json').write_text(json.dumps(records,indent=2)+'\n');print(name,round(r['cpu'],4),round(r['wall'],4),r['sql'],flush=True)
for fixture in sorted(pathlib.Path('/Users/jeremy/orca/artifacts/async-family-comparison/retained-after/edge-fixtures').iterdir()):corpus('production','edge-'+fixture.name,corpus=fixture)
for arm in bins:corpus(arm,'warmup')
for rep in range(3):
 for arm in (['baseline','production'] if rep%2==0 else ['production','baseline']):corpus(arm,'paired'+str(rep))
for arm in bins:corpus(arm,'profile',True)
summary={}
for arm in bins:
 rows=[r for r in records if r['arm']==arm and r['label'].startswith('paired')]
 summary[arm]={key:statistics.median(r[key] for r in rows) for key in ['cpu','wall','processCpu','processWall','sql']}
summary['pairs']=[{key:next(r[key] for r in records if r['label']=='paired'+str(rep) and r['arm']=='baseline')-next(r[key] for r in records if r['label']=='paired'+str(rep) and r['arm']=='production') for key in ['cpu','wall','processCpu','processWall']} for rep in range(3)]
(root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary,indent=2),flush=True)
