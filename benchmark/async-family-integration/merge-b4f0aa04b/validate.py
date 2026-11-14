import os,pathlib,subprocess,json,re,time,resource
root=pathlib.Path(__file__).resolve().parent
build=root.parent/'merge-b4f0aa04b-build.log'
binary=re.findall(r'Executable unittests src/lib.rs \(([^)]+)\)',build.read_text())[-1]
env={k:v for k,v in os.environ.items() if not k.startswith('RERAIL_')}
env.update(RUST_MIN_STACK='67108864',RERAIL_THREADS='4',RERAIL_CORPUS_MEMO='off',RERAIL_CROSS_MEMO='off',RERAIL_TAPE_CACHE=str(root.parent/'early-demands/production/early-cache.sqlite'))
checks=['facts::core::','tape::tests::','facts::vtab::syntax::','logic::plan::compose::tests::','logic::profile::analysis','logic::native::setfix::posted_tests::','authored_rr_batched_positive_documents_preserve_all_four_labels']
checks+=['logic::rr::tests::'+t for t in ['combined_programs_see_guests_by_their_own_policy','guests_that_do_not_parse_follow_the_policy','string_annotations_are_guests_at_their_host_lines','match_all_captures_spans_over_a_nodes_text_and_over_captures','match_all_captures_spans_over_source_text_and_symbols_over_symbols','requested_source_and_line_text_remain_exact_without_copying_large_subtrees','python_split_lines_are_lossless_and_document_local','lf_lines_keep_other_separators_and_the_trailing_empty_line','definition_ids_keep_other_definitions_rows_across_a_body_edit']]
records=[]
def run(label,test,e,ignored=False):
 command=['nice','-n','19',binary,test,'--nocapture','--test-threads=1']+(['--ignored'] if ignored else [])
 before=resource.getrusage(resource.RUSAGE_CHILDREN);start=time.perf_counter()
 with (root/(label+'.log')).open('w') as f:p=subprocess.run(command,env=e,stdout=f,stderr=subprocess.STDOUT,timeout=600)
 after=resource.getrusage(resource.RUSAGE_CHILDREN);log=(root/(label+'.log')).read_text()
 m=re.search(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',log)
 assert p.returncode==0 and m and int(m[1])>0 and int(m[2])==0, label+'\n'+log[-5000:]
 records.append(dict(label=label,passed=int(m[1]),ignored=int(m[3]),cpu=after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime,wall=time.perf_counter()-start,command=command))
 (root/'validation.json').write_text(json.dumps(records,indent=2)+'\n')
 print('PASS',label,m[1],flush=True)
for t in checks:run('test-'+t.replace(':','_'),t,env)
for t in ['projection_axes_and_token_spans_are_demanded_without_renumbering','authored_rr_batched_positive_documents_preserve_all_four_labels']:
 e=env.copy();e['RERAIL_NODE_IDS']='preorder';run('preorder-'+t,t,e)
for fixture in sorted((root.parent/'retained-after/edge-fixtures').iterdir()):
 label='edge-'+fixture.name;e=env.copy();e.update(RERAIL_SHARED_CORPUS=str(fixture),RERAIL_RR_OUT=str(root/label),RERAIL_RR_REPORT=str(root/(label+'.json')))
 run(label,'complete_async_family_authored_rr_corpus_profile',e,True)
 data=json.loads((root/(label+'.json')).read_text());old=json.loads((root.parent/'retained-after'/(label+'.json')).read_text());assert data['keys']==old['keys'],label+' finding parity'
e=env.copy();e.update(RERAIL_SHARED_CORPUS=str(root.parent/'corpus-fce4dc72'),RERAIL_RR_OUT=str(root/'corpus'),RERAIL_RR_REPORT=str(root/'corpus.json'),RERAIL_PROFILE=str(root/'corpus/profile'))
run('corpus','complete_async_family_authored_rr_corpus_profile',e,True)
data=json.loads((root/'corpus.json').read_text());assert data['report']['files']==5725 and data['keys']==[]
print('CORPUS',data['cpuNs']/1e9,data['wallNs']/1e9,sum(p[4] for p in data['report']['phases']),sum(s['vmSteps'] for s in data['profile']['statements']),flush=True)
