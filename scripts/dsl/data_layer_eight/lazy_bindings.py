# SPDX-License-Identifier: Apache-2.0
"""Module bindings, store contexts and native-cursor roles for lazy analysis."""
from data_layer_six.common import S


def callable_target_sql():
    return """CASE WHEN c.callee_kind='identifier' THEN coalesce(
      (SELECT i.module||'.'||i.name FROM imports i WHERE i.__path=c.__path AND i.local=c.callee AND i.kind='import_from_statement' AND i.fn=c.fn ORDER BY i.__start DESC LIMIT 1),
      (SELECT i.module||'.'||i.name FROM imports i WHERE i.__path=c.__path AND i.local=c.callee AND i.kind='import_from_statement' AND i.fn IS NULL ORDER BY i.__start DESC LIMIT 1),
      replace(substr(c.__path,1,length(c.__path)-3),'/','.')||'.'||coalesce((SELECT a.resolved FROM module_aliases a WHERE a.__path=c.__path AND a.name=c.callee),c.callee))
      WHEN c.callee_kind='attribute' AND receiver.kind='identifier' AND receiver.text IN ('self','cls') AND f.lexical_parent IS NOT NULL THEN
        replace(substr(c.__path,1,length(c.__path)-3),'/','.')||'.'||(SELECT cls.qualname FROM declaration_contexts cls WHERE cls.__path=f.__path AND cls.id=f.lexical_parent)||'.'||
          coalesce((SELECT a.target FROM class_method_aliases a WHERE a.__path=f.__path AND a.cls=f.lexical_parent AND a.name=c.callee ORDER BY a.__start DESC LIMIT 1),c.callee)
      WHEN c.callee_kind='attribute' AND receiver.kind='identifier' THEN (SELECT i.name||'.'||c.callee FROM imports i WHERE i.__path=c.__path AND i.kind='import_statement' AND i.fn IS NULL AND i.local=receiver.text ORDER BY i.__start DESC LIMIT 1) END"""


def conn_owner_sql(alias='c'):
    return """EXISTS(SELECT 1 FROM attributes attr WHERE attr.__path={c}.__path AND attr.attr='_conn' AND (
      EXISTS(SELECT 1 FROM arguments a WHERE a.__path={c}.__path AND a.call={c}.id AND a.position=0 AND attr.__start>=a.__start AND attr.__end<=a.__end)
      OR EXISTS(SELECT 1 FROM keywords k WHERE k.__path={c}.__path AND k.call={c}.id AND k.name IN ('owner','txn','tx','conn','connection') AND attr.__start>=k.__start AND attr.__end<=k.__end)))""".format(c=alias)


def apply(stages):
    aliases = [
      S('class_method_aliases',"SELECT a.__path,d.id cls,d.qualname,a.target_text name,a.expr_text target,a.__start FROM assignments a JOIN nodes stmt ON stmt.__path=a.__path AND stmt.id=a.parent JOIN declaration_contexts d ON d.__path=stmt.__path AND d.kind='class_definition' AND stmt.parent=d.body_kind||':'||d.body_start||':'||d.body_end WHERE a.target_kind='identifier' AND a.expr_kind='identifier' AND a.type_start IS NULL",('__path','cls','name')),
      S('module_alias_bindings', """SELECT a.__path,a.target_text name,
        CASE WHEN a.expr_kind='identifier' THEN a.expr_text WHEN a.expr_kind='attribute' THEN attr.attr
          WHEN a.expr_kind='call' AND call.callee_kind='identifier' THEN call.callee END target
        FROM assignments a JOIN nodes expression ON expression.__path=a.__path AND expression.id=a.parent
        JOIN nodes module ON module.__path=expression.__path AND module.id=expression.parent AND module.kind='module'
        LEFT JOIN attributes attr ON attr.__path=a.__path AND attr.id=a.value
        LEFT JOIN calls call ON call.__path=a.__path AND call.id=a.value
        WHERE a.target_kind='identifier' AND a.type_start IS NULL
        AND NOT EXISTS(SELECT 1 FROM assignments later JOIN nodes e ON e.__path=later.__path AND e.id=later.parent
          WHERE later.__path=a.__path AND later.target_text=a.target_text AND later.target_kind='identifier'
          AND later.type_start IS NULL AND e.parent=module.id AND later.__start>a.__start)""", ('__path','name')),
      S('module_aliases', """WITH RECURSIVE resolution(__path,root,token,seen) AS (
        SELECT __path,name,name,','||name||',' FROM module_alias_bindings
        UNION ALL SELECT r.__path,r.root,b.target,r.seen||b.target||',' FROM resolution r
          JOIN module_alias_bindings b ON b.__path=r.__path AND b.name=r.token
          WHERE b.target IS NOT NULL AND instr(r.seen,','||b.target||',')=0),
        terminal AS (SELECT r.__path,r.root,CASE WHEN b.target IS NOT NULL THEN b.target ELSE r.token END token
          FROM resolution r LEFT JOIN module_alias_bindings b ON b.__path=r.__path AND b.name=r.token
          WHERE b.target IS NULL OR instr(r.seen,','||b.target||',')>0)
        SELECT r.__path,r.root name,coalesce((SELECT i.module||'.'||i.name FROM imports i
          WHERE i.__path=r.__path AND i.fn IS NULL AND i.kind='import_from_statement' AND i.local=r.token),r.token) resolved
        FROM terminal r""", ('__path','name')),
      S('native_cursor_parameters', """WITH RECURSIVE annotation(__path,fn,name,node) AS (
        SELECT __path,fn,name,annotation FROM parameters WHERE annotation IS NOT NULL
        UNION ALL SELECT a.__path,a.fn,a.name,child.id FROM annotation a
          JOIN nodes n ON n.__path=a.__path AND n.id=a.node
          JOIN nodes child ON child.__path=n.__path AND child.id=CASE WHEN n.kind='subscript' THEN n.value_kind||':'||n.value_start||':'||n.value_end
            ELSE (SELECT first.id FROM nodes first WHERE first.__path=n.__path AND first.parent=n.id AND first.ordinal=0 LIMIT 1) END
          WHERE n.kind IN ('type','generic_type','subscript'))
        SELECT a.__path,a.fn,a.name FROM annotation a JOIN nodes n ON n.__path=a.__path AND n.id=a.node
        JOIN imports i ON i.__path=a.__path AND i.fn IS NULL AND (
          n.kind='identifier' AND i.kind='import_from_statement' AND i.local=n.text
          OR n.kind='attribute' AND i.kind='import_statement' AND i.local=n.object_source)
        WHERE CASE WHEN n.kind='identifier' THEN i.module||'.'||i.name ELSE i.name||'.'||n.attribute_source END
          IN ('psycopg.Cursor','psycopg.AsyncCursor','sqlite3.Cursor')""", ('__path','fn','name')),
    ]
    index = next(i for i,s in enumerate(stages) if s['name']=='terminal_values')
    stages[index:index] = aliases
    terminal = next(s for s in stages if s['name']=='terminal_values')
    terminal['query'] = terminal['query'].replace("coalesce((SELECT i.module||'.'||i.name FROM imports i", "coalesce((SELECT a.resolved FROM module_aliases a WHERE a.__path=n.__path AND a.name=n.text),(SELECT i.module||'.'||i.name FROM imports i")
    flags = next(s for s in stages if s['name']=='function_flags')
    resolved = "rx_extract(coalesce((SELECT a.resolved FROM module_aliases a WHERE a.__path=d.__path AND a.name=d.name),i.module||'.'||i.name,d.name),'[^.]+$',0)"
    flags['query'] = flags['query'].replace('coalesce(i.name,d.name)', resolved).replace('i.local=d.name WHERE', 'i.local=d.name AND i.fn IS NULL WHERE')
    stages.insert(stages.index(flags),S('execution_context_classes',"""WITH RECURSIVE heads(__path,cls,node) AS (
      SELECT c.__path,c.id,b.id FROM declaration_contexts c JOIN nodes args ON args.__path=c.__path AND args.parent=c.id AND args.kind='argument_list' JOIN nodes b ON b.__path=args.__path AND b.parent=args.id WHERE c.kind='class_definition'
      UNION ALL SELECT h.__path,h.cls,n.function_kind||':'||n.function_start||':'||n.function_end FROM heads h JOIN nodes n ON n.__path=h.__path AND n.id=h.node AND n.kind='call')
      SELECT DISTINCT h.__path,h.cls FROM heads h JOIN nodes n ON n.__path=h.__path AND n.id=h.node WHERE n.kind='identifier' AND n.text GLOB '*ExecutionContext' OR n.kind='attribute' AND n.attribute_source GLOB '*ExecutionContext'""",('__path','cls')))
    flags['query']=flags['query'].replace('SELECT f.*,',"""SELECT f.*,
      EXISTS(SELECT 1 FROM execution_context_classes ctx JOIN declaration_contexts cls ON cls.__path=ctx.__path AND cls.id=ctx.cls WHERE cls.__path=f.__path AND cls.region_start<f.region_start AND cls.__end>=f.__end) AND f.name IN ('execute_sync','execute_async') execution_context,
      EXISTS(SELECT 1 FROM nodes stmt LEFT JOIN nodes value ON value.__path=stmt.__path AND value.parent=stmt.id AND value.ordinal=0 LEFT JOIN calls call ON call.__path=value.__path AND call.id=value.id WHERE stmt.__path=f.__path AND stmt.parent=f.body_kind||':'||f.body_start||':'||f.body_end AND stmt.kind='raise_statement' AND (CASE WHEN value.kind='call' THEN call.callee WHEN value.kind='attribute' THEN value.attribute_source WHEN value.kind='identifier' THEN value.text END GLOB '*NotImplementedError' OR CASE WHEN value.kind='call' THEN call.callee WHEN value.kind='attribute' THEN value.attribute_source WHEN value.kind='identifier' THEN value.text END GLOB '*LazyPurityError')
        AND NOT EXISTS(SELECT 1 FROM nodes prior WHERE prior.__path=stmt.__path AND prior.parent=stmt.parent AND prior.__start<stmt.__start AND prior.kind!='comment' AND NOT(prior.kind='expression_statement' AND EXISTS(SELECT 1 FROM nodes doc WHERE doc.__path=prior.__path AND doc.parent=prior.id AND doc.kind IN ('string','concatenated_string'))))) tombstone,""",1)
    targets = next(s for s in stages if s['name']=='call_targets')
    targets['query'] = "SELECT c.__path,c.fn,c.__start,c.id,"+callable_target_sql()+" target FROM owned_calls c JOIN function_flags f ON f.__path=c.__path AND f.id=c.fn LEFT JOIN nodes receiver ON receiver.__path=c.__path AND receiver.id=c.receiver"
    index=stages.index(targets)+1
    stages[index:index]=[
      S('delegate_values',"SELECT source.__path,source.fn,source.id,source.__start,source.__end,k.__start keyword_start,node.kind callee_kind,CASE WHEN node.kind='identifier' THEN node.text ELSE node.attribute_source END callee,node.id callee_id,node.object_kind||':'||node.object_start||':'||node.object_end receiver FROM owned_calls source JOIN keywords k ON k.__path=source.__path AND k.call=source.id AND k.name IN ('sync_resolve','async_resolve') JOIN nodes node ON node.__path=k.__path AND node.id=k.value WHERE (source.resolved='defer_lazy_result' OR source.resolved GLOB '*.defer_lazy_result') AND node.kind IN ('identifier','attribute')",('__path','fn')),
      S('delegate_targets',"SELECT c.__path,c.fn,c.id,c.__start,c.__end,c.keyword_start,"+callable_target_sql()+" target FROM delegate_values c JOIN function_flags f ON f.__path=c.__path AND f.id=c.fn LEFT JOIN nodes receiver ON receiver.__path=c.__path AND receiver.id=c.receiver",('__path','fn')),
    ]
    owned = next(s for s in stages if s['name']=='owned_nodes')
    index = stages.index(owned)
    stages.insert(index,S('assignment_targets','SELECT __path,target id FROM assignments',('__path','id')))
    owned['query'] = owned['query'].replace("WHERE n.kind!='class_definition'", "WHERE n.kind!='class_definition' AND NOT EXISTS(SELECT 1 FROM assignment_targets target WHERE target.__path=n.__path AND target.id=n.id)")
    events = next(s for s in stages if s['name']=='function_events')
    events['query'] = """SELECT e.__path,e.fn,e.id,e.__start,e.__end,e.category,
      CASE WHEN e.category='plan' AND EXISTS(SELECT 1 FROM classified_calls c
        JOIN nodes receiver ON receiver.__path=c.__path AND receiver.id=c.receiver AND receiver.kind='identifier'
        JOIN native_cursor_parameters p ON p.__path=c.__path AND p.fn=c.fn AND p.name=receiver.text
        WHERE c.__path=e.__path AND c.fn=e.fn AND c.id=e.id AND c.callee IN ('fetchone','fetchmany','fetchall'))
        THEN 'native_cursor.'||e.detail ELSE e.detail END detail,
        e.category='plan' AND EXISTS(SELECT 1 FROM classified_calls c WHERE c.__path=e.__path AND c.id=e.id AND """+conn_owner_sql()+") uses_conn FROM ("+events['query']+") e"
    rollups = next(s for s in stages if s['name']=='rollups')
    rollups['query'] = rollups['query'].replace('e.category,min(', 'category.kind category,min(')
    rollups['query'] = rollups['query'].replace("WHERE e.category IN ('plan','intent','grammar','native','dolt')", "JOIN (SELECT 'plan' source,'plan' kind UNION ALL SELECT 'plan','orchestration' UNION ALL SELECT 'intent','intent' UNION ALL SELECT 'grammar','grammar' UNION ALL SELECT 'native','native' UNION ALL SELECT 'dolt','dolt') category ON category.source=e.category AND (category.kind!='orchestration' OR e.detail NOT GLOB 'native_cursor.*')")
    rollups['query'] = rollups['query'].replace('GROUP BY r.__path,r.root,e.category', 'GROUP BY r.__path,r.root,category.kind')
