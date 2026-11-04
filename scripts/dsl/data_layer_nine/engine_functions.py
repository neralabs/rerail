# SPDX-License-Identifier: Apache-2.0
"""Function-scoped bootstrap invariants, preserving nested subtree multiplicity."""
from .common import *

SCHEMA="f.basename='schema_transaction.py'"
COORD="f.basename='bootstrap_coordinator.py'"
PRIMITIVE="substr(f.name,1,17)='ensure_bootstrap_' AND substr(f.name,-5)='_lazy'"

def rows(c):
 result=[]
 def add(code,msg,where,helper,source='function_nodes f JOIN a n ON n.__path=f.__path AND n.id BETWEEN f.id AND f.last',node='n',sub='n.bfs',engine=False):
  if engine:where='f.checked AND ('+where+')'
  result.append(row(code,msg,source,where,node=node,order=f"printf('%020d/%06d/%024.6f',f.bfs,{helper*1000},({sub}))"))
 calls='function_nodes f JOIN calls n ON n.__path=f.__path AND n.id BETWEEN f.id AND f.last'
 selfnode='function_nodes f';N='f'
 # _collect_pure_lazy_body_violations
 for code,key,kind in [('BEI005','EXECUTABLE_BOOTSTRAP_NAMES','executable bootstrap'),('BEI023','FORBIDDEN_PURE_LAZY_BOUNDARY_CALLS','boundary/materialization')]:
  add(code,f"'@pure_lazy function '||f.name_repr||' calls {kind} surface '||(SELECT name_repr FROM a x WHERE x.__path=n.__path AND x.id=n.callee_id)",pure('f')+f' AND n.callee IN {literals(c[key])}',1,calls,engine=True)
 executable=f"work.callee IN {literals(c['EXECUTABLE_BOOTSTRAP_NAMES'])} OR substr(work.callee,-7)='_direct' OR substr(work.callee,1,6)='build_'"
 add('BEI006',q('LazyResult.of(...) must not wrap executable bootstrap work'),pure('f')+" AND n.callee_kind='Attribute' AND n.callee='of' AND n.receiver_kind='Name' AND n.receiver_name='LazyResult' AND EXISTS(SELECT 1 FROM calls work WHERE work.__path=n.__path AND work.parent=n.id AND work.field='args' AND work.position=0 AND ("+executable+'))',1,calls,sub='n.bfs+0.01',engine=True)
 # _collect_fake_executor_violations
 add('BEI007',q('bootstrap application executor must use typed execution authority, not discard conn'),"f.name='execute_application_plan' AND n.kind='Delete'",2,source="function_nodes f JOIN a n ON n.__path=f.__path AND n.id BETWEEN f.id AND f.last JOIN a target ON target.__path=n.__path AND target.parent=n.id AND target.field='targets' AND target.kind='Name' AND target.name='conn'",sub='n.bfs+target.position*0.0001',engine=True)
 guard="EXISTS(SELECT 1 FROM a branch JOIN a test ON test.__path=branch.__path AND test.parent=branch.id AND test.field='test' JOIN a operand ON operand.__path=test.__path AND operand.parent=test.id AND operand.field IN ('left','comparators') WHERE branch.__path=n.__path AND branch.kind='If' AND n.id BETWEEN branch.id AND branch.last AND test.kind='Compare' AND operand.kind='Attribute' AND operand.name='ZERO_APPLICATION_TERMINALS' AND branch.id BETWEEN f.id AND f.last)"
 add('BEI007',q('bootstrap application executor returns synthetic zero mutation facts'),"f.name='execute_application_plan' AND n.callee='BootstrapExecutionFacts' AND NOT "+guard+" AND value.callee='BootstrapMutationCounts' AND (SELECT count(*) FROM a k JOIN a v ON v.__path=k.__path AND v.parent=k.id AND v.field='value' WHERE k.__path=value.__path AND k.parent=value.id AND k.field='keywords' AND v.kind='Constant' AND (v.value_kind IN ('int','float','complex') AND CAST(v.value AS REAL)=0 OR v.value_kind='bool' AND v.value='False'))>=4",2,calls+" JOIN a kw ON kw.__path=n.__path AND kw.parent=n.id AND kw.field='keywords' AND kw.name='mutation_counts' JOIN calls value ON value.__path=kw.__path AND value.parent=kw.id AND value.field='value'",sub='n.bfs+kw.position*0.0001',engine=True)
 # _collect_coordinator_publication_tail_violations
 publish=COORD+" AND f.name='_enqueue_application_schema_lazy'"
 add('BEI016',q('metadata publication must consume produced BootstrapApplicationPlan refs'),publish+" AND n.callee='publish_bootstrap_metadata_lazy' AND v.kind='Tuple' AND NOT "+child('v','elts',alias='elt'),3,calls+" JOIN a k ON k.__path=n.__path AND k.parent=n.id AND k.field='keywords' AND k.name='dependency_refs' JOIN a v ON v.__path=k.__path AND v.parent=k.id AND v.field='value'",sub='n.bfs+k.position*0.0001',engine=True)
 add('BEI012',q('cold application final tail must depend on planned metadata/proof publication'),publish+" AND n.callee='bootstrap_schema_tail_lazy' AND n.line=(SELECT max(t.line) FROM calls t WHERE t.__path=f.__path AND t.id BETWEEN f.id AND f.last AND t.callee='bootstrap_schema_tail_lazy') AND n.id=(SELECT min(t.id) FROM calls t WHERE t.__path=f.__path AND t.id BETWEEN f.id AND f.last AND t.callee='bootstrap_schema_tail_lazy' AND t.line=n.line) AND NOT EXISTS(SELECT 1 FROM calls p WHERE p.__path=f.__path AND p.id BETWEEN f.id AND f.last AND p.callee='publish_bootstrap_metadata_lazy' AND p.line<=n.line) OR "+publish+" AND n.callee='bootstrap_schema_tail_lazy' AND n.id=(SELECT min(t.id) FROM calls t WHERE t.__path=f.__path AND t.id BETWEEN f.id AND f.last AND t.callee='bootstrap_schema_tail_lazy' AND t.line=(SELECT max(x.line) FROM calls x WHERE x.__path=f.__path AND x.id BETWEEN f.id AND f.last AND x.callee='bootstrap_schema_tail_lazy')) AND EXISTS(SELECT 1 FROM calls p WHERE p.__path=f.__path AND p.id BETWEEN f.id AND f.last AND p.callee='publish_bootstrap_metadata_lazy' AND p.line>n.line)",3,calls,sub='1000000',engine=True)
 # _collect_lifecycle_skip_violations
 effect=child('n','test',alias='test',condition=desc('test',condition="n.kind='Attribute' AND n.name='APPLY_DDL_LOCKDOWN' AND "+child('n','value',alias='value',condition="value.kind='Name' AND value.name='BootstrapLifecycleEffectKind'")))
 calls_in_body="EXISTS(SELECT 1 FROM a stmt JOIN calls call ON call.__path=stmt.__path AND call.id BETWEEN stmt.id AND stmt.last WHERE stmt.__path=n.__path AND stmt.parent=n.id AND stmt.field='body' AND call.callee='apply_bootstrap_ddl_lockdown_effect')"
 add('BEI015',q('required DDL lockdown lifecycle effect must execute, not be skipped'),COORD+" AND f.name='apply_lifecycle_effects' AND n.kind='If' AND "+effect+" AND "+child('n','body',condition="v.kind='Continue'")+" AND NOT "+calls_in_body,4,engine=True)
 # _collect_pipeline_lifecycle_violations
 add('BEI019',q('cold bootstrap lifecycle effects must run at the explicit post-application boundary'),COORD+" AND f.name='plan_application' AND n.callee='lower_bootstrap_lifecycle_effects'",5,calls,engine=True)
 add('BEI019',q('cold application tail must not execute lifecycle effects as ServerPlan callbacks'),publish+" AND n.callee='bootstrap_schema_tail_lazy' AND "+child('n','keywords',condition="v.name='lifecycle_effects'"),5,calls,engine=True)
 # _collect_schema_tail_boundary_violations
 tail=SCHEMA+" AND f.name='bootstrap_schema_tail_lazy'"
 add('BEI024',q('bootstrap_schema_tail_lazy must accept the BootstrapApplicationPlan-produced tail ref'),tail+" AND NOT EXISTS(SELECT 1 FROM a p WHERE p.__path=f.__path AND p.fn=f.id AND p.kind='arg' AND p.field IN ('args','kwonlyargs') AND p.name='produced_ref')",6,selfnode,node='f',sub='0',engine=True)
 add('BEI024',q('bootstrap_schema_tail_lazy must publish facts only; materialization/flush belongs to the application boundary'),tail+" AND n.callee IN ('_materialize_schema_operation_records','_flush_deferred_create_table_ddl_batch','flush_schema_plan','flush_schema_plan_async')",6,calls,engine=True)
 # _collect_application_ref_graph_violations and install-time lowering
 materialize=COORD+" AND f.name='_materialize_application_lazy_results'"
 add('BEI025',q('application install must consume BootstrapApplicationPlan dependency refs, not reset them to ()'),materialize+" AND n.kind='Assign' AND "+child('n','targets',condition="v.kind='Name' AND v.name='schema_dependency_refs'")+" AND "+child('n','value',condition="v.kind='Tuple' AND NOT "+child('v','elts',alias='elt')),7,engine=True)
 add('BEI025',q('application tail installation must pass application_plan.application_tail_ref as produced_ref'),materialize+" AND n.callee='bootstrap_schema_tail_lazy' AND NOT "+child('n','keywords',condition="v.name='produced_ref'"),7,calls,engine=True)
 add('BEI026',q('contributor primitive lowering must be represented in BootstrapDataPrimitivePlan before install'),materialize+" AND n.kind='Attribute' AND "+child('n','value',condition="v.kind='Name' AND v.name='BootstrapDataPrimitiveKind'"),8,engine=True)
 # _collect_transition_replay_validation_violations
 required=['artifact_hash','backend_facts','backend_fingerprint','desired_hash','observed_hash']
 missing="(SELECT '['||group_concat(quote(name),', ')||']' FROM ("+' UNION ALL '.join('SELECT '+q(x)+' AS name' for x in required)+") required WHERE NOT EXISTS(SELECT 1 FROM a child WHERE child.__path=f.__path AND child.id BETWEEN f.id AND f.last AND child.kind IN ('Name','Attribute') AND child.name=required.name))"
 add('BEI027',"'transition replay planning must validate backend/observed/desired/artifact identity; missing '||"+missing,COORD+" AND f.name='_transition_replay_plan_for_observation' AND "+missing+" IS NOT NULL",9,selfnode,node='f',sub='0',engine=True)
 # Runner and public API signatures; body checker in pinned source intentionally returns [].
 for helper,names,code,prefix in [(10,c['BOOTSTRAP_RUNNER_NAMES'],'BEI029',None),(12,c['BOOTSTRAP_LAZY_API_NAMES'],'BEI033',"'bootstrap API '||f.name_repr")]:
  scope=COORD+f' AND f.name IN {literals(names)}'
  for rank,(condition,suffix) in enumerate([(f"substr(f.name,-5)!='_lazy'",' must be an explicit _lazy function'),('NOT '+pure('f'),' must be decorated with @pure_lazy'),('NOT '+child('f','returns',condition="substr(v.annotation,1,11)='LazyResult['"),' must return LazyResult[...]')]):
   if rank==0 and prefix:continue
   add(code,(prefix+'||'+q(suffix)) if prefix else q('bootstrap runner'+suffix),scope+' AND '+condition,helper,selfnode,node='f',sub=str(rank),engine=True)
 missing_api="(SELECT '['||group_concat(quote(name),', ')||']' FROM ("+' UNION ALL '.join('SELECT '+q(x)+' name' for x in sorted(c['BOOTSTRAP_LAZY_API_NAMES']))+") required WHERE NOT EXISTS(SELECT 1 FROM calls call WHERE call.__path=f.__path AND call.id BETWEEN f.id AND f.last AND call.callee=required.name))"
 wrappers=COORD+f" AND f.name IN {literals(c['BOOTSTRAP_RUN_WRAPPER_NAMES'])}"
 add('BEI034',"'bootstrap wrapper must call the public lazy APIs; missing '||"+missing_api,wrappers+' AND '+missing_api+' IS NOT NULL',13,selfnode,node='f',sub='0',engine=True)
 add('BEI034',"'bootstrap wrapper must not call eager/internal surface '||(SELECT name_repr FROM a x WHERE x.__path=n.__path AND x.id=n.callee_id)",wrappers+f" AND n.callee IN {literals(c['BOOTSTRAP_RUN_WRAPPER_FORBIDDEN_CALLS'])}",13,calls,engine=True)
 return result
