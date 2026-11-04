# SPDX-License-Identifier: Apache-2.0
"""Bootstrap engine contracts expressed over shared, typed syntax relations."""
from .common import *
from .engine_functions import rows as function_rows
from .engine_shapes import rows as shape_rows
from .engine_nodes import rows as node_rows

DEFAULT=['l3200_metadata/bootstrap_state.py','l3200_metadata/bootstrap_coordinator.py']

def candidate(root):
 c=constants(root/'scripts/lint_bootstrap_engine_invariants.py')
 stages=[S('configuration',{'$option':'parameters','escape':'sql','template':'SELECT {} data'}),
 S('entry',{'$option':'entryPoint','escape':'sql','template':'SELECT {} name'}),
 S('effective_source',"SELECT source.__path,coalesce(json_extract(configuration.data,'$.rawSources.'||json_quote(source.__path)),source.source) source FROM source CROSS JOIN configuration"),
 S('file_scope',f"""SELECT source.__path,rx_replace(source.__path,'^.*/','') basename,
 EXISTS(SELECT 1 FROM configuration,json_each(coalesce(json_extract(data,'$.enginePaths'),{q(json.dumps(DEFAULT))})) p WHERE p.value=source.__path) OR EXISTS(SELECT 1 FROM configuration,json_each(coalesce(json_extract(data,'$.bootstrapPaths'),{q(json.dumps(DEFAULT))})) p WHERE p.value=source.__path) engine,
 EXISTS(SELECT 1 FROM configuration,json_each(coalesce(json_extract(data,'$.bootstrapPaths'),{q(json.dumps(DEFAULT))})) p WHERE p.value=source.__path) bootstrap
 FROM effective_source source CROSS JOIN entry WHERE entry.name='_collect_fake_executor_violations' OR EXISTS(SELECT 1 FROM configuration,json_each(coalesce(json_extract(data,'$.bootstrapPaths'),{q(json.dumps(DEFAULT))})) p WHERE p.value=source.__path)
 OR rx_matches(source.__path,'(^|/)(schema_transaction|sql_connection|bootstrap_interrogation)\\.py$') OR instr(rx_replace(source.__path,'^.*/',''),'bootstrap')>0
 OR rx_matches(source.source,'Bootstrap|bootstrap|schema_transaction|materialized_pipeline|physical_table|reconciliation')""",('__path',)),
 S('engine_nodes','SELECT a.*,s.basename,s.engine,s.bootstrap FROM a JOIN file_scope s ON s.__path=a.__path',('__path','id'),('__path','kind','name')),
 S('engine_calls','SELECT c.*,s.basename,s.engine,s.bootstrap FROM calls c JOIN file_scope s ON s.__path=c.__path',('__path','id'),('__path','callee')),
 S('function_nodes',"SELECT f.*,s.basename,s.engine,s.bootstrap,s.engine OR s.basename='schema_transaction.py' AND f.name IN ('bootstrap_interrogation_lazy','bootstrap_interrogation_request_no_io','bootstrap_schema_tail_lazy') OR entry.name='_collect_fake_executor_violations' AS checked FROM functions f JOIN file_scope s ON s.__path=f.__path CROSS JOIN entry CROSS JOIN configuration WHERE entry.name!='_collect_fake_executor_violations' OR json_extract(configuration.data,'$.functionLine') IS NULL OR f.line=json_extract(configuration.data,'$.functionLine')",('__path','id'))]
 # Raw scans use each surface's first occurrence. They are separate from AST
 # name matches, and apply to comments and strings as in the pinned source.
 raw=[]
 for family,(key,code,label) in enumerate([('DELETED_COLD_BOOTSTRAP_SURFACES','BEI035','cold-bootstrap'),('DELETED_SCHEMA_SIDECAR_SURFACES','BEI035','schema sidecar'),('DELETED_NON_BATCH_BOOTSTRAP_SCHEMA_SURFACES','BEI046','non-batch bootstrap schema')]):
  for ordinal,surface in enumerate(sorted(c[key])):
   raw.append('('+','.join([str(family),str(ordinal),q(surface),q(code),q(label)])+')')
 stages.extend([S('deleted_surfaces',"SELECT column1 family,column2 ordinal,column3 surface,column4 issue,'deleted '||column5||' surface '||quote(column3)||' must not be reintroduced' message FROM (VALUES "+','.join(raw)+')'),S('raw_matches',"""SELECT n.__path,n.__start,n.__end,length(substr(source.source,1,instr(source.source,d.surface)-1))-length(replace(substr(source.source,1,instr(source.source,d.surface)-1),char(10),''))+1 line,
 d.issue,d.message,d.family*1000+d.ordinal ordering,d.surface FROM a n JOIN file_scope s ON s.__path=n.__path JOIN effective_source source ON source.__path=n.__path JOIN deleted_surfaces d ON instr(source.source,d.surface)>0 WHERE n.kind='Module' AND (d.family<2 OR s.basename='schema_transaction.py')""")])
 raw_query="SELECT __path path,__start start_byte,__end end_byte,line,issue,message,surface||':'||issue identity,-1 phase,ordering,'' symbol,'' sort_name FROM raw_matches"
 queries=[raw_query,*function_rows(c),*shape_rows(c),*node_rows(c)]
 packets=[]
 for query in queries:
  if not packets or len(' UNION ALL '.join(packets[-1]+[query]).encode())>14500:packets.append([])
  packets[-1].append(query)
 for i,packet in enumerate(packets):stages.append(S('diagnostics_'+str(i),' UNION ALL '.join(packet)))
 all_query=' UNION ALL '.join('SELECT * FROM diagnostics_'+str(i) for i in range(len(packets)))
 # Cross-file enum coverage selects the last function of each name, just as the
 # original dictionary does. The first matching enum class defines its members.
 coverage_stages=[
 S('install_functions',"SELECT f.* FROM functions f WHERE f.__path='l3200_metadata/bootstrap_coordinator.py' AND f.name IN ('_data_primitive_plan_for_plan','_enqueue_application_schema_lazy','_install_schema_primitive_contract','_install_data_mutation_contract','_install_versioning_mutation_contract') AND NOT EXISTS(SELECT 1 FROM functions later WHERE later.__path=f.__path AND later.name=f.name AND later.bfs>f.bfs)",('__path','id')),
 S('installed_kinds',"SELECT DISTINCT n.name FROM install_functions f JOIN a n ON n.__path=f.__path AND n.id BETWEEN f.id AND f.last WHERE n.kind='Attribute' AND "+child('n','value',condition="v.kind='Name' AND v.name='BootstrapDataPrimitiveKind'"),('name',)),
 S('declared_kinds',"""SELECT t.name,t.id,t.position FROM a cls JOIN a declaration ON declaration.__path=cls.__path AND declaration.parent=cls.id AND declaration.field='body' AND declaration.kind IN ('Assign','AnnAssign') JOIN a t ON t.__path=declaration.__path AND t.parent=declaration.id AND t.field IN ('target','targets') AND t.kind='Name'
 WHERE cls.__path='l3200_metadata/bootstrap_data_plan.py' AND cls.kind='ClassDef' AND cls.name='BootstrapDataPrimitiveKind' AND cls.bfs=(SELECT min(x.bfs) FROM a x WHERE x.__path=cls.__path AND x.kind='ClassDef' AND x.name=cls.name)""",('name',))]
 coverage=row('BEI038',"'BootstrapDataPrimitiveKind.'||k.name||' has no lazy install family'",'a n JOIN declared_kinds k',"n.__path='l3200_metadata/bootstrap_coordinator.py' AND n.kind='Module' AND NOT EXISTS(SELECT 1 FROM installed_kinds i WHERE i.name=k.name)",order='k.id',phase=1)
 query={'$option':'entryPoint','cases':{'_collect_fake_executor_violations':"SELECT * FROM ("+all_query+") WHERE issue='BEI007'"},'default':all_query}
 item=clause('bootstrap-invariants',stages,query,partition=False)
 disabled={'$option':'entryPoint','cases':{'_collect_primitive_install_coverage_violations':FALSE},'default':TRUE}
 item['relational']['inputs']['gate']['evidence']['query']['ast']['rule']=A(K('module'),disabled)
 item['relational']['requireNonemptyInput']='gate'
 item['relational']['inputs']['source']=E(A(K('module'),P('SOURCE'),disabled),{'source':'capture.SOURCE'})
 cov=clause('primitive-install-coverage',coverage_stages,coverage,partition=False)
 cov['relational']['inputs']['gate']['evidence']['query']['ast']['rule']=A(K('module'),{'$option':'entryPoint','cases':{'_collect_primitive_install_coverage_violations':TRUE,'all':TRUE},'default':FALSE})
 cov['relational']['requireNonemptyInput']='gate'
 invalid=E(A(K('module'),P('SOURCE')),{'source':'capture.SOURCE'})
 invalid['evidence']['query']['ast']['syntax']='invalid'
 scope_query=next(stage['query'] for stage in stages if stage['name']=='file_scope').replace('FROM effective_source source','FROM invalid_source source')
 syntax={'id':'bootstrap-source-syntax','relational':{'inputs':{'invalid_source':invalid},'stages':[S('configuration',{'$option':'parameters','escape':'sql','template':'SELECT {} data'}),S('entry',{'$option':'entryPoint','escape':'sql','template':'SELECT {} name'}),S('invalid_scope',scope_query)],
 'query':"SELECT i.__path path,i.__start start_byte,i.__end end_byte,'NATIVE-SYNTAX' issue,i.__path identity FROM invalid_source i JOIN invalid_scope s ON s.__path=i.__path",'key':['identity'],'code':'issue'},'violation':{'code':'NATIVE-SYNTAX','message':'Python source cannot be parsed','anchor':'witness.span'}}
 result=document('bootstrap_engine_invariants',[item,cov,syntax],['l3200_metadata/**/*.py','l2000_connection/schema_transaction.py','l2055_lazy_result/purity.py','impl/common/l2200_sql_dialect/{sql_connection,sql_core_support_free_functions,sql_core_support_mixin,sql_execution_mixin,sql_schema_services,sql_bootstrap_metadata_services}.py','impl/common/l2070_server_plan/steps/bootstrap_interrogation.py','impl/common/l7245_versioned_backends/{generic_sparse,staged_table_triggers}.py','impl/read_through/connection.py','data_layer_testing/helpers/{bootstrap,bootstrap_manifest}.py'])
 result['scope']['exclude']=['**/'+part+'/**' for part in ['.git','.venv','venv','__pycache__','tmp']]
 return result
