# SPDX-License-Identifier: Apache-2.0
"""Semantic-step storage and same-class authentication helper reachability."""
from data_layer_six.common import S,q,literals
from data_layer_eight.relations import clause
from data_layer_eight.authority import source_paths

def candidate(source,policy):
    module=source/'scripts/lint_authentication_plan_purity.py'
    p=policy('authentication_plan_purity',source_paths(module,'DEFAULT_SOURCES'))
    p['clauses'].append({'id':'entry','define':{'options':{'oracleAnalyzer':{'type':'boolean','default':False},'entryPoint':{'type':'string','default':'scan_sources'}}}})
    # Breadth-first CPython class selection: statement body/decorator wrappers
    # do not contribute AST depth. Class names are a file-wide namespace here.
    stages=[
      S('depths',"""WITH RECURSIVE depths(__path,id,depth) AS (
        SELECT __path,id,0 FROM nodes WHERE parent IS NULL
        UNION ALL SELECT n.__path,n.id,d.depth+CASE WHEN n.kind IN ('block','decorated_definition','else_clause','finally_clause','expression_statement') THEN 0 ELSE 1 END
        FROM depths d JOIN edges e ON e.__path=d.__path AND e.src=d.id JOIN nodes n ON n.__path=e.__path AND n.id=e.dst) SELECT * FROM depths""",('__path','id')),
      S('selected_classes',"SELECT * FROM (SELECT c.*,row_number() OVER(PARTITION BY c.__path,c.name ORDER BY d.depth DESC,c.__start DESC) chosen FROM classes c JOIN depths d ON d.__path=c.__path AND d.id=c.id) WHERE chosen=1",('__path','name')),
      S('base_heads',"""WITH RECURSIVE heads(__path,cls,id) AS (
        SELECT c.__path,c.id,b.id FROM selected_classes c JOIN nodes a ON a.__path=c.__path AND a.parent=c.id AND a.kind='argument_list' JOIN nodes b ON b.__path=a.__path AND b.parent=a.id
        UNION ALL SELECT h.__path,h.cls,n.id FROM heads h JOIN nodes x ON x.__path=h.__path AND x.id=h.id JOIN nodes n ON n.__path=x.__path AND (x.kind='parenthesized_expression' AND n.parent=x.id OR x.kind='call' AND n.id=x.function_kind||':'||x.function_start||':'||x.function_end))
        SELECT h.__path,h.cls,CASE WHEN n.kind='identifier' THEN n.text WHEN n.kind='attribute' THEN n.attribute_source END AS name FROM heads h JOIN nodes n ON n.__path=h.__path AND n.id=h.id WHERE n.kind IN ('identifier','attribute')""",('__path','cls','name')),
      S('semantic_classes',"""WITH RECURSIVE semantic(__path,name) AS (
        SELECT __path,name FROM selected_classes WHERE name GLOB '*PlanStep'
        UNION SELECT c.__path,c.name FROM selected_classes c JOIN base_heads b ON b.__path=c.__path AND b.cls=c.id WHERE b.name IN ('ServerPlanStep','NonPublishingStep','SqlProducingStep','SqlRelationPublisherStep','SqlScalarPublisherStep')
        UNION SELECT c.__path,c.name FROM selected_classes c JOIN base_heads b ON b.__path=c.__path AND b.cls=c.id JOIN semantic s ON s.__path=b.__path AND s.name=b.name) SELECT * FROM semantic""",('__path','name')),
      S('step_methods',"SELECT m.* FROM methods m JOIN semantic_classes s ON s.__path=m.__path AND s.name=m.class_name",('__path','id')),
      S('parameter_types',"""SELECT p.__path,p.fn,p.name,coalesce(n.text,'') annotation FROM parameters p LEFT JOIN nodes n ON n.__path=p.__path AND n.id=p.annotation WHERE p.kind IN ('typed_parameter','typed_default_parameter')""",('__path','fn','name')),
      S('self_stores',"""SELECT n.__path,n.id,n.__start,n.__end,n.type_source annotation,n.right_kind value_kind,n.right_source value,a.attr,fn.id method FROM nodes n JOIN attributes a ON a.__path=n.__path AND a.id=n.left_kind||':'||n.left_start||':'||n.left_end
        JOIN step_methods fn ON fn.__path=n.__path AND fn.__start<n.__start AND fn.__end>=n.__end WHERE n.kind='assignment' AND a.object_text='self'""",('__path','id')),
      S('store_contracts',"""SELECT s.*,coalesce(s.annotation,'')||' '||CASE WHEN s.value_kind='identifier' THEN coalesce((SELECT annotation FROM parameter_types p WHERE p.__path=s.__path AND p.fn=s.method AND p.name=s.value),'') WHEN s.value_kind='call' THEN coalesce((SELECT callee FROM calls c WHERE c.__path=s.__path AND c.id=s.value_kind||':'||(SELECT right_start FROM nodes n WHERE n.__path=s.__path AND n.id=s.id)||':'||(SELECT right_end FROM nodes n WHERE n.__path=s.__path AND n.id=s.id)),'') ELSE '' END AS types FROM self_stores s"""),
      S('last_methods',"SELECT * FROM (SELECT m.*,row_number() OVER(PARTITION BY __path,cls,name ORDER BY __start DESC) chosen FROM methods m) WHERE chosen=1",('__path','cls','name')),
      S('method_calls',"SELECT m.__path,m.cls,m.name,c.callee FROM last_methods m JOIN calls c ON c.__path=m.__path AND m.__start<c.__start AND m.__end>=c.__end",('__path','cls','name')),
      S('entry_reach',"""WITH RECURSIVE reachable(__path,cls,entry,name) AS (
        SELECT __path,cls,name,name FROM last_methods WHERE instr(class_name,'ReadThroughWriteTransaction')>0 AND name IN ('__enter__','__aenter__')
        UNION SELECT r.__path,r.cls,r.entry,m.name FROM reachable r JOIN method_calls c ON c.__path=r.__path AND c.cls=r.cls AND c.name=r.name JOIN last_methods m ON m.__path=c.__path AND m.cls=c.cls AND m.name=c.callee WHERE m.name NOT IN ('_establish_legacy_write_authority','_establish_native_write_authority')) SELECT * FROM reachable""",('__path','cls','entry')),
      S('function_decorators',"""SELECT f.__path,f.id,coalesce(c.callee,a.attr,n.text) name FROM functions f JOIN nodes w ON w.__path=f.__path AND w.id=f.parent AND w.kind='decorated_definition' JOIN nodes d ON d.__path=w.__path AND d.parent=w.id AND d.kind='decorator' JOIN nodes n ON n.__path=d.__path AND n.parent=d.id LEFT JOIN calls c ON c.__path=n.__path AND c.id=n.id LEFT JOIN attributes a ON a.__path=n.__path AND a.id=n.id""",('__path','id','name')),
    ]
    emit=lambda table,where,code:f"SELECT n.__path path,n.__start start_byte,n.__end end_byte,{q(code)} issue,n.id identity FROM {table} n WHERE {where}"
    def suffixes(values):return r'(?:^|[\s\[\]|,])[^\s\[\]|,]*(?:'+'|'.join(values)+r')(?:$|[\s\[\]|,])'
    rows=[emit('store_contracts',"attr IN ('_client','client','_connection','connection','_callback','callback','_registry','registry') OR rx_matches(types,"+q(suffixes(['Client','Connection','Transaction','Callback','Registry','Transport']))+")",'APP001'),
      emit('store_contracts',"instr(attr,'batched_value')>0 OR instr(attr,'response')>0 OR instr(attr,'runtime_result')>0 OR rx_matches(types,"+q(suffixes(['Response','RuntimeResult']))+")",'APP002'),
      emit('last_methods',"name IN ('__enter__','__aenter__') AND EXISTS(SELECT 1 FROM entry_reach r JOIN method_calls c ON c.__path=r.__path AND c.cls=r.cls AND c.name=r.name WHERE r.__path=n.__path AND r.cls=n.cls AND r.entry=n.name AND (instr(c.callee,'bind_cache_to_current_authority')>0 OR instr(c.callee,'adopt_authenticated_remote_session')>0 OR instr(c.callee,'resolve')>0 AND (instr(c.callee,'auth')>0 OR instr(c.callee,'authority')>0 OR instr(c.callee,'credential')>0 OR instr(c.callee,'admission')>0)))",'APP003'),
      emit('functions',"(name GLOB '*_sync' OR name GLOB '*_async') AND (instr(lower(name),'auth')>0 OR instr(lower(name),'authority')>0 OR instr(lower(name),'credential')>0 OR instr(lower(name),'admission')>0) AND NOT EXISTS(SELECT 1 FROM function_decorators d WHERE d.__path=n.__path AND d.id=n.id AND (d.name IN ('legacy_sync','legacy_async') OR d.name IN ('direct_legacy_sync','direct_legacy_async') AND EXISTS(SELECT 1 FROM calls c WHERE c.__path=n.__path AND c.__start>n.__start AND c.__end<=n.__end AND c.callee GLOB '*_lazy')))",'APP004')]
    p['clauses'].append(clause('authentication-contracts',stages,' UNION ALL '.join(rows)))
    return p
