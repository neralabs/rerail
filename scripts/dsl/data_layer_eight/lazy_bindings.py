# SPDX-License-Identifier: Apache-2.0
"""Module bindings, store contexts and native-cursor roles for lazy analysis."""
from data_layer_six.common import S


def apply(stages):
    aliases = [
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
    targets = next(s for s in stages if s['name']=='call_targets')
    targets['query'] = targets['query'].replace("THEN coalesce((SELECT i.module||'.'||i.name FROM imports i", "THEN coalesce((SELECT replace(substr(c.__path,1,length(c.__path)-3),'/','.')||'.'||a.resolved FROM module_aliases a WHERE a.__path=c.__path AND a.name=c.callee),(SELECT i.module||'.'||i.name FROM imports i")
    targets['query'] = targets['query'].replace("i.kind='import_statement' AND i.local=receiver.text", "i.kind='import_statement' AND i.fn IS NULL AND i.local=receiver.text")
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
        THEN 'native_cursor.'||e.detail ELSE e.detail END detail FROM ("""+events['query']+") e"
    rollups = next(s for s in stages if s['name']=='rollups')
    rollups['query'] = rollups['query'].replace('e.category,min(', 'category.kind category,min(')
    rollups['query'] = rollups['query'].replace("WHERE e.category IN ('plan','intent','grammar','native','dolt')", "JOIN (SELECT 'plan' source,'plan' kind UNION ALL SELECT 'plan','orchestration' UNION ALL SELECT 'intent','intent' UNION ALL SELECT 'grammar','grammar' UNION ALL SELECT 'native','native' UNION ALL SELECT 'dolt','dolt') category ON category.source=e.category AND (category.kind!='orchestration' OR e.detail NOT GLOB 'native_cursor.*')")
    rollups['query'] = rollups['query'].replace('GROUP BY r.__path,r.root,e.category', 'GROUP BY r.__path,r.root,category.kind')
