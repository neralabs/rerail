# SPDX-License-Identifier: Apache-2.0
"""SQL surface diagnostics over shared syntax, lexical owners and typed slices."""
import re
from data_layer_six.common import S,literals,q
from data_layer_generate_batch_seven import constant
from data_layer_eight.relations import clause


def candidate(source,policy):
    module=source/'scripts/lint_lazy_sql_surface.py';value=lambda name:constant(module,name)
    p=policy('lazy_sql_surface',['impl/**/*.py','data_layer/**/*.py','_internal/**/*.py','l[0-9][0-9][0-9][0-9]_*/**/*.py'])
    p['clauses'].append({'id':'entry','define':{'options':{'entryPoint':{'type':'string','default':'scan_repo'}}}})
    stages=[
      S('terminal_names',"""WITH RECURSIVE head(__path,root,node) AS (
        SELECT __path,id,id FROM nodes WHERE kind IN ('call','attribute','identifier','as_pattern','parenthesized_expression')
        UNION ALL SELECT h.__path,h.root,c.id FROM head h JOIN nodes n ON n.__path=h.__path AND n.id=h.node JOIN nodes c ON c.__path=n.__path AND c.parent=n.id AND c.ordinal=0 WHERE n.kind IN ('as_pattern','parenthesized_expression')
        UNION ALL SELECT h.__path,h.root,n.function_kind||':'||n.function_start||':'||n.function_end FROM head h JOIN nodes n ON n.__path=h.__path AND n.id=h.node WHERE n.kind='call')
        SELECT h.__path,h.root,coalesce(a.attr,n.text) name FROM head h JOIN nodes n ON n.__path=h.__path AND n.id=h.node LEFT JOIN attributes a ON a.__path=n.__path AND a.id=n.id WHERE n.kind IN ('attribute','identifier')""",('__path','root')),
      S('authority_regions',"""SELECT w.__path,w.id,w.__start,w.__end FROM nodes w WHERE w.kind='with_statement' AND EXISTS(SELECT 1 FROM nodes clause JOIN nodes item ON item.__path=clause.__path AND item.parent=clause.id JOIN nodes value ON value.__path=item.__path AND value.parent=item.id JOIN terminal_names t ON t.__path=value.__path AND t.root=value.id WHERE clause.__path=w.__path AND clause.parent=w.id AND clause.kind='with_clause' AND item.kind='with_item' AND t.name='explicit_execution_authority' AND NOT EXISTS(SELECT 1 FROM nodes attr WHERE attr.__path=value.__path AND attr.kind='attribute' AND attr.__start>=value.__start AND attr.__end<=value.__end))""",('__path','__start','__end')),
      S('peeled',"""WITH RECURSIVE peel(__path,root,id) AS (SELECT __path,id,id FROM nodes UNION ALL SELECT p.__path,p.root,c.id FROM peel p JOIN nodes n ON n.__path=p.__path AND n.id=p.id JOIN nodes c ON c.__path=n.__path AND c.parent=n.id AND c.kind!='comment' WHERE n.kind IN ('type','type_parameter','parenthesized_expression') AND (SELECT count(*) FROM nodes s WHERE s.__path=n.__path AND s.parent=n.id AND s.kind!='comment')=1) SELECT p.__path,p.root,p.id,n.kind,n.text FROM peel p JOIN nodes n ON n.__path=p.__path AND n.id=p.id WHERE n.kind NOT IN ('type','type_parameter','parenthesized_expression')""",('__path','root')),
      S('subscripts',"""SELECT g.__path,g.id,g.__start,g.__end,h.text name,p.id argument,p.kind argument_kind,p.text argument_text FROM nodes g JOIN nodes head ON head.__path=g.__path AND head.parent=g.id AND head.ordinal=0 JOIN peeled h ON h.__path=head.__path AND h.root=head.id JOIN nodes sub ON sub.__path=g.__path AND sub.parent=g.id AND sub.ordinal=1 JOIN peeled p ON p.__path=sub.__path AND p.root=sub.id WHERE g.kind IN ('generic_type','subscript') AND h.kind='identifier' AND (SELECT count(*) FROM nodes sibling WHERE sibling.__path=g.__path AND sibling.parent=g.id AND sibling.kind!='comment')=2""",('__path','id')),
      S('bad_annotations',"""SELECT __path,id,text rendered FROM nodes WHERE kind='identifier' AND text IN ('object','DatabaseRow') OR kind='none'
       UNION SELECT __path,id,'list[DatabaseRow]' FROM subscripts WHERE name='list' AND argument_kind='identifier' AND argument_text='DatabaseRow'
       UNION SELECT n.__path,n.id,'DatabaseRow | None' FROM nodes n JOIN nodes l ON l.__path=n.__path AND l.parent=n.id AND l.ordinal=0 JOIN peeled lp ON lp.__path=l.__path AND lp.root=l.id JOIN nodes r ON r.__path=n.__path AND r.parent=n.id AND r.ordinal=1 JOIN peeled rp ON rp.__path=r.__path AND rp.root=r.id WHERE n.kind IN ('union_type','binary_operator') AND lp.kind='identifier' AND lp.text='DatabaseRow' AND rp.kind='none' AND rx_matches(substr(n.text,l.__end-n.__start+1,r.__start-l.__end),'\\|')""",('__path','id')),
      S('diagnostics',"SELECT c.__path,c.__start,c.__end,c.id,'connection grammar lazy terminal call: '||c.callee message FROM calls c WHERE c.callee_kind='attribute' AND c.callee IN "+literals(value('FORBIDDEN_CONNECTION_GRAMMAR_LAZY_METHODS'))+
        " UNION ALL SELECT c.__path,c.__start,c.__end,c.id,'ServerPlan '||c.callee||'() outside execution boundary' FROM calls c JOIN terminal_names r ON r.__path=c.__path AND r.root=c.receiver WHERE c.callee_kind='attribute' AND c.callee IN ('execute','execute_async') AND (trim(lower(r.name),'_') IN ('plan','plan_executor') OR trim(lower(r.name),'_') GLOB '*plan') AND c.__path NOT IN "+literals(value('ALLOWED_PLAN_EXECUTE_FILES'))+" AND NOT EXISTS(SELECT 1 FROM authority_regions a WHERE a.__path=c.__path AND a.__start<=c.__start AND a.__end>=c.__end)"+
        " UNION ALL SELECT a.__path,a.__start,a.__end,a.id,'step compile() reads ctx.'||a.attr FROM attributes a JOIN nodes object ON object.__path=a.__path AND object.id=a.object JOIN ownership o ON o.__path=a.__path AND o.id=a.id JOIN functions f ON f.__path=o.__path AND f.id=o.fn WHERE object.kind='identifier' AND object.text='ctx' AND f.name='compile' AND a.attr IN "+literals(value('FORBIDDEN_COMPILE_CONTEXT_ATTRS'))+
        " UNION ALL SELECT f.__path,f.__start,f.__end,f.id,'SyncSqlConnection defines forbidden grammar terminal: '||f.name FROM functions f JOIN ownership o ON o.__path=f.__path AND o.id=f.id JOIN classes c ON c.__path=o.__path AND c.id=o.cls WHERE c.name='SyncSqlConnection' AND f.name IN "+literals(value('FORBIDDEN_CONNECTION_GRAMMAR_LAZY_METHODS'))+
        " UNION ALL SELECT s.__path,s.__start,s.__end,s.id,'untyped LazyResult['||a.rendered||']' FROM subscripts s JOIN bad_annotations a ON a.__path=s.__path AND a.id=s.argument WHERE s.name='LazyResult' AND rx_matches(s.__path,"+q('^(?:'+'|'.join(map(re.escape,value('TYPED_LAZY_RESULT_REL_PREFIXES')))+')')+")"),
    ]
    trigger='LazyResult|SyncSqlConnection|compile|execute|execute_async|explicit_execution_authority|grammar_lazy|legacy_async|legacy_sync'
    where=" FROM diagnostics d WHERE EXISTS(SELECT 1 FROM nodes n WHERE n.__path=d.__path AND n.kind='module' AND rx_matches(n.text,"+q(trigger)+")) AND NOT rx_matches(__path,'(?:^|/)(?:tests|conform)(?:/|$)')"
    item=clause('sql-surface',stages,'SELECT __path path,__start start_byte,__end end_byte,message issue,id identity,message'+where)
    item['relational']['message']='message';p['clauses'].append(item);return p
