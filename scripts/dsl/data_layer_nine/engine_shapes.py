# SPDX-License-Identifier: Apache-2.0
from .common import *
from .engine_functions import SCHEMA,COORD,PRIMITIVE

def rows(c):
 result=[]
 def add(code,msg,where,helper,source='function_nodes f JOIN a n ON n.__path=f.__path AND n.id BETWEEN f.id AND f.last',node='n',sub='n.bfs'):
  result.append(row(code,msg,source,where,node=node,order=f"printf('%020d/%06d/%024.6f',f.bfs,{helper*1000},({sub}))"))
 calls='function_nodes f JOIN calls n ON n.__path=f.__path AND n.id BETWEEN f.id AND f.last'
 f='function_nodes f'
 primitive=SCHEMA+' AND '+PRIMITIVE
 # Legacy lowering helpers, including helpers outside engine files.
 coordinator=COORD+" AND f.name='_enqueue_application_schema_lazy'"
 schema=SCHEMA+" AND f.name IN ('ensure_commit_triggers_lazy','ensure_queue_lazy','ensure_logged_table_lazy','ensure_table_event_trigger_lazy','ensure_large_column_metadata_schema_lazy')"
 add('BEI014',"CASE WHEN f.basename='bootstrap_coordinator.py' THEN 'coordinator must use LazySchemaTransaction primitive lowering surfaces' ELSE 'LazySchemaTransaction primitive surfaces must not delegate to legacy coordinator helpers' END",f'({coordinator} OR {schema}) AND n.callee IN {literals(c["LEGACY_PRIMITIVE_HELPER_NAMES"])}',14,calls)
 # Primitive stubs, including every matching marker in a string.
 add('BEI036',q('_unsupported_bootstrap_schema_surface_lazy is a temporary tripwire and must be deleted'),SCHEMA+" AND f.name='_unsupported_bootstrap_schema_surface_lazy'",15,f,node='f',sub='0')
 add('BEI036',"f.name||' still delegates to the unsupported bootstrap primitive tripwire'",primitive+" AND n.callee='_unsupported_bootstrap_schema_surface_lazy'",15,calls)
 for rank,marker in enumerate(frozenset(c['BOOTSTRAP_PRIMITIVE_STUB_STRINGS'])):
  add('BEI036',"f.name||"+q(' still contains placeholder primitive lowering text '+repr(marker)),primitive+" AND n.kind='Constant' AND n.value_kind='str' AND instr(n.value,"+q(marker)+')>0',15,sub=f'n.bfs+{rank}*0.01')
 shape=SCHEMA+f" AND ({PRIMITIVE} OR f.name IN {literals(c['SCHEMA_TRANSACTION_LAZY_PLAN_SURFACES'])})"
 for rank,(condition,suffix) in enumerate([('NOT '+pure('f'),' must be decorated with @pure_lazy'),('NOT '+child('f','returns',condition="substr(v.annotation,1,11)='LazyResult['"),' must return LazyResult[...]')]):
  add('BEI039',"'schema-transaction lazy bootstrap surface '||f.name_repr||"+q(suffix),shape+' AND '+condition,16,f,node='f',sub=str(rank))
 for rank,name in enumerate(['produced_ref','dependency_refs']):
  add('BEI039',"'bootstrap primitive lowerer '||f.name_repr||"+q(' must accept keyword-only '+name),primitive+" AND NOT EXISTS(SELECT 1 FROM a p WHERE p.__path=f.__path AND p.fn=f.id AND p.kind='arg' AND p.field='kwonlyargs' AND p.name="+q(name)+')',16,f,node='f',sub=str(2+rank))
 discarded="(SELECT '['||group_concat(quote(name),', ')||']' FROM (SELECT DISTINCT name FROM a t WHERE t.__path=n.__path AND t.parent=n.id AND t.field='targets' AND t.kind='Name' AND t.name IN ('produced_ref','dependency_refs') ORDER BY name))"
 add('BEI040',"'bootstrap primitive lowerer '||f.name_repr||' must preserve contract refs, not discard '||"+discarded,primitive+" AND n.kind='Delete' AND "+discarded+' IS NOT NULL',16)
 # Application install shape, including deleted scaffolding.
 add('BEI043',q('build_execution_plan_lazy must call the application install runner directly, not materialize lazy-result buckets'),COORD+" AND f.name='build_execution_plan_lazy' AND n.callee IN ('_materialize_application_lazy_results','_install_application_lazy_results')",17,calls)
 add('BEI043',q('_enqueue_application_schema_lazy must build and return the terminal LazyResult directly, not fetch a preinstalled tail'),coordinator+" AND n.kind='Attribute' AND n.name IN ('lazy_results','application_tail_result')",17)
 add('BEI043',q('_enqueue_application_schema_lazy must be the single installer that calls bootstrap_schema_tail_lazy'),coordinator+' AND NOT '+desc('f',table='calls',condition="n.callee='bootstrap_schema_tail_lazy'"),17,f,node='f',sub='1000000')
 add('BEI043',"'deleted application install scaffold '||f.name_repr||' must be removed or folded into _enqueue_application_schema_lazy'",COORD+" AND f.name IN ('_materialize_application_lazy_results','_install_application_lazy_results')",17,f,node='f',sub='0')
 lowerer=SCHEMA+" AND (substr(f.name,1,17)='ensure_bootstrap_' OR f.name IN ('_build_ddl_effect_lazy','_enqueue_bootstrap_dml_statements_lazy','assert_bootstrap_sql_assertions_lazy','bootstrap_schema_tail_lazy'))"
 add('BEI044',"'bootstrap lowerer '||f.name_repr||' must collect work into the fused terminal payload, not emit ordinary SQL step via '||(SELECT name_repr FROM a x WHERE x.__path=n.__path AND x.id=n.callee_id)",lowerer+f" AND n.callee IN {literals(c['BOOTSTRAP_APPLICATION_SQL_STEP_EMITTERS'])} AND n.callee!=f.name",18,calls)
 add('BEI037',"f.name||' must use primitive dispatch, not direct installer '||(SELECT name_repr FROM a x WHERE x.__path=n.__path AND x.id=n.callee_id)",COORD+f" AND f.name IN ('_materialize_application_lazy_results','_install_application_lazy_results') AND n.callee IN {literals(c['COORDINATOR_DIRECT_INSTALL_CALLS'])}",19,calls)
 interrogation=f"({COORD} AND f.name='execute_interrogation_plan' OR {SCHEMA} AND f.name='bootstrap_interrogation_lazy')"
 forbidden={'lazy_tuple','physical_table_exists_many_lazy','columns_exist_many_lazy','physical_indexes_many_lazy','bootstrap_schema_state_facts_lazy','bootstrap_manifest_facts_lazy','bootstrap_proof_facts_lazy','bootstrap_seed_facts_lazy','bootstrap_backend_facts_lazy','bootstrap_transition_artifact_facts_lazy','bootstrap_ddl_lockdown_facts_lazy'}
 add('BEI011',q('bootstrap interrogation must use BootstrapInterrogationStep rows, not compose helper refs directly'),interrogation+f' AND n.callee IN {literals(forbidden)}',20,calls)
 add('BEI013',q('execute_runtime_rows must not synthesize physical table/column/index facts'),"f.basename='bootstrap_interrogation.py' AND f.name='execute_runtime_rows' AND n.kind='Attribute' AND n.name IN ('TABLE','COLUMN','INDEX') AND "+child('n','value',condition="v.kind='Name' AND v.name='BootstrapInterrogationRowKind'"),21)
 add('BEI045',q('ensure_versioning_domain_schema must be the sole owner of versioning infra tables; do not emit ENSURE_TABLE duplicates'),"f.name='ensure_versioning_domain_schema' AND n.callee='ensure_tables'",22,calls)
 add('BEI046',"'ensure bootstrap schema primitives must delegate to batch installers, not call non-batch lowering surface '||(SELECT name_repr FROM a x WHERE x.__path=n.__path AND x.id=n.callee_id)",primitive+f" AND n.callee IN {literals(c['FORBIDDEN_ENSURE_BOOTSTRAP_SCHEMA_LOWERING_CALLS'])}",23,calls)
 production="instr('/'||f.__path||'/','/tests/')=0 AND instr('/'||f.__path||'/','/data_layer_testing/')=0 AND (f.bootstrap OR instr(f.basename,'bootstrap')>0)"
 counterpart="CASE WHEN substr(f.name,-6)='_async' THEN substr(f.name,1,length(f.name)-6)||'_lazy' WHEN substr(f.name,-5)='_sync' THEN substr(f.name,1,length(f.name)-5)||'_lazy' ELSE f.name||'_lazy' END"
 add('BEI047',"'bootstrap surface '||f.name_repr||' must be lazy-only; delete sync/async variants and keep the _lazy entrypoint'",production+" AND instr(f.name,'bootstrap')>0 AND substr(f.name,-5)!='_lazy' AND NOT "+pure('f')+" AND (f.kind='AsyncFunctionDef' OR substr(f.name,-6)='_async' OR EXISTS(SELECT 1 FROM functions counterpart WHERE counterpart.__path=f.__path AND counterpart.name="+counterpart+'))',24,f,node='f',sub='0')
 return result
