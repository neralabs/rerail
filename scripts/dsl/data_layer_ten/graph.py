# SPDX-License-Identifier: Apache-2.0
from .common import *
MIXIN='l7200_versioned_transactions/versioned_lazy_mixin.py'
TRANSACTIONS='l7200_versioned_transactions/versioned_transactions.py'
SQL='l7200_versioned_transactions/graph_lazy_sql.py'
INTENT='impl/common/l2070_server_plan/steps/intent_graph.py'


def candidate(root):
    c=constants(root/'scripts/lint_graph_lazy_sql_plan.py')
    stages=[S('graph_classes',"SELECT n.* FROM classes n WHERE n.fn IS NULL AND n.cls IS NULL AND NOT EXISTS(SELECT 1 FROM classes earlier WHERE earlier.__path=n.__path AND earlier.name=n.name AND earlier.fn IS NULL AND earlier.cls IS NULL AND earlier.id<n.id)",('__path','name')),
      S('graph_method_candidates',"SELECT f.*,owner.name owner_name FROM functions f JOIN classes owner ON owner.__path=f.__path AND owner.id=f.cls WHERE f.fn IS NULL AND owner.fn IS NULL AND owner.cls IS NULL",('__path','owner_name','name')),
      S('graph_methods',"SELECT m.* FROM graph_method_candidates m WHERE NOT EXISTS(SELECT 1 FROM graph_method_candidates later WHERE later.__path=m.__path AND later.owner_name=m.owner_name AND later.name=m.name AND later.id>m.id)",('__path','owner_name','name')),
      S('top_definitions',"SELECT n.__path,n.id,n.__start,n.__end,n.line,n.name,CASE n.kind WHEN 'ClassDef' THEN 'class' ELSE 'function' END symbol_kind FROM a n WHERE n.kind IN ('ClassDef','FunctionDef','AsyncFunctionDef') AND n.fn IS NULL AND n.cls IS NULL UNION ALL SELECT n.__path,n.id,n.__start,n.__end,n.line,t.name,CASE n.kind WHEN 'TypeAlias' THEN 'typealias' ELSE 'assign' END FROM a n JOIN a t ON t.__path=n.__path AND t.parent=n.id AND t.field IN ('targets','name') AND t.kind='Name' WHERE n.kind IN ('Assign','TypeAlias') AND n.fn IS NULL AND n.cls IS NULL",('__path','name','id')),
      S('top_symbols',"SELECT n.*,(SELECT min(first.id) FROM top_definitions first WHERE first.__path=n.__path AND first.name=n.name) insertion FROM top_definitions n WHERE NOT EXISTS(SELECT 1 FROM top_definitions later WHERE later.__path=n.__path AND later.name=n.name AND later.id>n.id)",('__path','name'))]
    rows=[]
    def emit(code,message,source,where,order,phase=0,node='n'):
        rows.append(row(code,message,source,where,order=str(order),phase=phase,node=node))
    def exists(owner,path,alias='n'):
        return f"EXISTS(SELECT 1 FROM graph_classes cls WHERE cls.__path={alias}.__path AND cls.name={q(owner)})"
    def missing_class(path,owner,code,message,order):
        emit(code,q(message),'a n',f"n.kind='Module' AND n.__path={q(path)} AND NOT {exists(owner,path)}",order)
    mixin='VersionedReadTransactionLazyMixin'
    emit('GLSP-001',"'Banned graph lazy implementation symbol `'||n.name||'` must remain deleted'",'top_symbols n',f"n.__path={q(MIXIN)} AND n.symbol_kind IN ('class','function') AND n.name IN {literals(c['BANNED_VERSIONED_LAZY_SYMBOLS'])} AND {exists(mixin,MIXIN)}",'n.insertion')
    missing_class(MIXIN,mixin,'GLSP-002',mixin+' class not found',0)
    emit('GLSP-003',q('`graph_lazy` must exist as a pure lazy graph entrypoint'),'graph_classes n',f"n.__path={q(MIXIN)} AND n.name={q(mixin)} AND NOT EXISTS(SELECT 1 FROM graph_methods m WHERE m.__path=n.__path AND m.owner_name=n.name AND m.name='graph_lazy')",0,phase=1)
    emit('GLSP-004',q('`graph_lazy` must be decorated with @pure_lazy'),'graph_methods n',f"n.__path={q(MIXIN)} AND n.owner_name={q(mixin)} AND n.name='graph_lazy' AND NOT EXISTS(SELECT 1 FROM single_decorators d WHERE d.__path=n.__path AND d.fn=n.id AND d.name='pure_lazy' AND d.kind IN ('Name','Attribute'))",0,phase=1)
    for i,name in enumerate(['graph_query_builder_lazy','graph_traverse_lazy']):emit('GLSP-003',q('Removed graph shortcut `'+name+'` must remain absent'),'graph_methods n',f"n.__path={q(MIXIN)} AND n.owner_name={q(mixin)} AND n.name={q(name)}",i,phase=2)
    required=sorted(['GraphLazyStepRegistrar','SqlGraphExecutionCtx','SqlPlanGraphQuery','SqlPlanGraphQueryBuilder','SqlPlanGraphTraversal'])
    for i,name in enumerate(required):missing_class(SQL,name,'GLSP-006','Required graph lazy SQL class `'+name+'` is missing',i)
    registrar='GraphLazyStepRegistrar'
    emit('GLSP-007',q('`GraphLazyStepRegistrar` must declare `__pure_lazy_surface__`'),'a n LEFT JOIN graph_classes cls ON cls.__path=n.__path AND cls.name=\'GraphLazyStepRegistrar\'',f"n.kind='Module' AND n.__path={q(SQL)} AND (cls.id IS NULL OR NOT EXISTS(SELECT 1 FROM a assignment JOIN classes owner ON owner.__path=assignment.__path AND owner.id=assignment.cls JOIN a t ON t.__path=assignment.__path AND t.parent=assignment.id AND t.field='targets' AND t.kind='Name' AND t.name='__pure_lazy_surface__' WHERE assignment.__path=n.__path AND assignment.kind='Assign' AND assignment.fn IS NULL AND owner.fn IS NULL AND owner.cls IS NULL AND owner.name={q(registrar)}))",0,phase=1)
    rows[-1]=rows[-1].replace('n.__start AS start_byte,n.__end AS end_byte,n.line,','n.__start AS start_byte,n.__end AS end_byte,coalesce(cls.line,1) line,',1)
    for i,name in enumerate(['execute_lazy','execute_one_lazy','count_lazy','exists_lazy','execute_bindings_lazy','execute_aggregate_lazy','execute_paths_lazy']):
        emit('GLSP-008',q('`SqlPlanGraphQueryBuilder.'+name+'` is missing'),'graph_classes n',f"n.__path={q(SQL)} AND n.name='SqlPlanGraphQueryBuilder' AND NOT EXISTS(SELECT 1 FROM graph_methods m WHERE m.__path=n.__path AND m.owner_name=n.name AND m.name={q(name)})",i,phase=2)
        emit('GLSP-009',q('`SqlPlanGraphQueryBuilder.'+name+'` must be decorated with @pure_lazy'),'graph_methods n',f"n.__path={q(SQL)} AND n.owner_name='SqlPlanGraphQueryBuilder' AND n.name={q(name)} AND NOT EXISTS(SELECT 1 FROM single_decorators d WHERE d.__path=n.__path AND d.fn=n.id AND d.name='pure_lazy' AND d.kind IN ('Name','Attribute'))",i,phase=2)
    owner='LazyVersionedReadTransaction'
    missing_class(TRANSACTIONS,owner,'GLSP-006',owner+' class not found',0)
    for i,(name,decorator) in enumerate([('graph','legacy_sync'),('graph_async','legacy_async')]):
        condition=f"n.__path={q(TRANSACTIONS)} AND n.owner_name={q(owner)} AND n.name={q(name)}"
        emit('GLSP-007',q('`'+name+'` must exist as a public delegate surface'),'graph_classes n',f"n.__path={q(TRANSACTIONS)} AND n.name={q(owner)} AND NOT EXISTS(SELECT 1 FROM graph_methods m WHERE m.__path=n.__path AND m.owner_name=n.name AND m.name={q(name)})",i,phase=1)
        emit('GLSP-010',q('`'+name+'` must be decorated with @'+decorator),'graph_methods n',condition+f" AND NOT EXISTS(SELECT 1 FROM single_decorators d WHERE d.__path=n.__path AND d.fn=n.id AND d.name={q(decorator)} AND d.kind IN ('Name','Attribute'))",i,phase=1)
        emit('GLSP-011',q('`'+name+'` must remain a legacy delegate shell'),'graph_methods n',condition+' AND NOT '+desc('n',alias='mention',condition="mention.kind IN ('Name','Attribute') AND mention.name IN ('_legacy_delegate_stub','_txn_legacy_delegate_body','_versioned_legacy_delegate_body')"),i+0.1,phase=1)
    for i,name in enumerate(['graph_query_builder','graph_query_builder_async','graph_traverse','graph_traverse_async']):emit('GLSP-007',q('Removed graph shortcut `'+name+'` must remain absent'),'graph_methods n',f"n.__path={q(TRANSACTIONS)} AND n.owner_name={q(owner)} AND n.name={q(name)}",i,phase=2)
    emit('GLSP-010',"'Banned interim graph terminal symbol `'||n.name||'` must remain deleted'",'top_symbols n',f"n.__path={q(INTENT)} AND n.name IN {literals(c['BANNED_INTENT_GRAPH_SYMBOLS'])}",'n.insertion')
    for i,name in enumerate(sorted(['ExecuteGraphNodesStep','ExecuteGraphOneStep','ExecuteGraphCountStep','ExecuteGraphExistsStep','ExecuteGraphBindingsStep','ExecuteGraphAggregateStep','ExecuteGraphPathsStep','_build_graph_execution_ctx'])):
        emit('GLSP-012',q('Required graph intent symbol `'+name+'` is missing'),'a n',f"n.kind='Module' AND n.__path={q(INTENT)} AND NOT EXISTS(SELECT 1 FROM top_symbols s WHERE s.__path=n.__path AND s.name={q(name)} AND s.symbol_kind IN ('class','function'))",i,phase=1)
    emit('GLSP-013',q('ExecuteGraphBindingsStep must declare cte_column_type_hints() for SQL-visible binding columns'),'graph_classes n',f"n.__path={q(INTENT)} AND n.name='ExecuteGraphBindingsStep' AND NOT EXISTS(SELECT 1 FROM graph_methods m WHERE m.__path=n.__path AND m.owner_name=n.name AND m.name='cte_column_type_hints')",0,phase=2)
    for i,name in enumerate(['SqlColumnTypeHints','_binding_node_sys_id_alias','_binding_edge_alias','uuid_columns','text_columns','integer_columns']):emit('GLSP-014',q('ExecuteGraphBindingsStep.cte_column_type_hints() must mention `'+name+'`'),'graph_methods n',f"n.__path={q(INTENT)} AND n.owner_name='ExecuteGraphBindingsStep' AND n.name='cte_column_type_hints' AND NOT "+desc('n',alias='mention',condition="mention.kind IN ('Name','Attribute') AND mention.name="+q(name)),i,phase=2)
    items=[]
    for index in range(0,len(rows),6):
        item=make_clause('graph-plan-contracts-'+str(index//6),stages,' UNION ALL '.join(rows[index:index+6]))
        item['relational']['line']='line'
        items.append(item)
    return make_document('graph_lazy_sql_plan',items,[MIXIN,TRANSACTIONS,SQL,INTENT])
