# SPDX-License-Identifier: Apache-2.0
"""Reusable Python declaration, lexical-context and call-edge relations."""
from data_layer_six.common import S


def stages():
    return [
      S('declarations',"SELECT n.*,n.name_source name,coalesce(w.__start,n.__start) region_start FROM nodes n LEFT JOIN nodes w ON w.__path=n.__path AND w.id=n.parent AND w.kind='decorated_definition' WHERE n.kind IN ('function_definition','class_definition')",('__path','id')),
      S('declaration_contexts',"""SELECT d.*,coalesce((SELECT group_concat(name,'.') FROM (SELECT name FROM declarations a WHERE a.__path=d.__path AND a.region_start<d.region_start AND a.__end>=d.__end ORDER BY a.region_start))||'.','')||d.name qualname,
        (SELECT a.id FROM declarations a WHERE a.__path=d.__path AND a.region_start<d.region_start AND a.__end>=d.__end ORDER BY a.region_start DESC LIMIT 1) lexical_parent FROM declarations d""",('__path','id'),('__path','name')),
      S('decorator_names',"""WITH RECURSIVE walk(__path,fn,decorator,node) AS (
        SELECT f.__path,f.id,d.id,x.id FROM declarations f JOIN nodes wrapper ON wrapper.__path=f.__path AND wrapper.id=f.parent AND wrapper.kind='decorated_definition' JOIN nodes d ON d.__path=wrapper.__path AND d.parent=wrapper.id AND d.kind='decorator' JOIN nodes x ON x.__path=d.__path AND x.parent=d.id
        UNION ALL SELECT w.__path,w.fn,w.decorator,c.callee_id FROM walk w JOIN calls c ON c.__path=w.__path AND c.id=w.node)
        SELECT w.__path,w.fn,w.decorator,w.node,coalesce(a.attr,n.text) name FROM walk w JOIN nodes n ON n.__path=w.__path AND n.id=w.node LEFT JOIN attributes a ON a.__path=n.__path AND a.id=n.id WHERE n.kind IN ('identifier','attribute')""",('__path','fn','name')),
      S('expression_roots',"""WITH RECURSIVE root(__path,expression,node,through_call) AS (SELECT __path,id,id,0 FROM nodes UNION ALL SELECT r.__path,r.expression,c.id,r.through_call OR n.kind='call' FROM root r JOIN nodes n ON n.__path=r.__path AND n.id=r.node JOIN nodes c ON c.__path=n.__path AND (c.id=n.object_kind||':'||n.object_start||':'||n.object_end AND n.kind='attribute' OR c.id=n.value_kind||':'||n.value_start||':'||n.value_end AND n.kind='subscript' OR c.id=n.function_kind||':'||n.function_start||':'||n.function_end AND n.kind='call' OR c.parent=n.id AND c.ordinal=0 AND n.kind IN ('parenthesized_expression','type'))) SELECT r.__path,r.expression,n.id,n.text name,r.through_call FROM root r JOIN nodes n ON n.__path=r.__path AND n.id=r.node WHERE n.kind='identifier'""",('__path','expression')),
      S('bound_names',"""WITH RECURSIVE bound(__path,assignment,node) AS (SELECT __path,id,target FROM assignments UNION ALL SELECT b.__path,b.assignment,c.id FROM bound b JOIN nodes n ON n.__path=b.__path AND n.id=b.node JOIN nodes c ON c.__path=n.__path AND c.parent=n.id WHERE n.kind IN ('tuple','list','pattern_list','tuple_pattern','list_pattern')) SELECT b.__path,b.assignment,n.id,n.text name FROM bound b JOIN nodes n ON n.__path=b.__path AND n.id=b.node WHERE n.kind='identifier'""",('__path','assignment'),('__path','name')),
      S('call_edges',"""SELECT DISTINCT c.__path,o.fn caller,t.id target FROM calls c JOIN ownership o ON o.__path=c.__path AND o.id=c.id JOIN declaration_contexts owner ON owner.__path=o.__path AND owner.id=o.fn JOIN expression_roots root ON root.__path=c.__path AND root.expression=c.callee_id JOIN declaration_contexts t ON t.__path=c.__path AND t.kind='function_definition' AND t.name=c.callee
        WHERE (c.callee_kind='identifier' AND (t.lexical_parent IS NULL OR t.lexical_parent=owner.lexical_parent) OR c.callee_kind='attribute' AND root.name IN ('self','cls') AND t.lexical_parent=owner.lexical_parent)
        AND NOT EXISTS(SELECT 1 FROM nodes n WHERE n.__path=c.__path AND n.kind='lambda' AND n.__start<c.__start AND n.__end>=c.__end)
        AND (root.name IN ('self','cls') OR NOT EXISTS(SELECT 1 FROM parameters p WHERE p.__path=c.__path AND p.fn=o.fn AND p.name=root.name) AND NOT EXISTS(SELECT 1 FROM assignments a JOIN bound_names b ON b.__path=a.__path AND b.assignment=a.id WHERE a.__path=c.__path AND a.fn=o.fn AND b.name=root.name))""",('__path','caller'),('__path','target')),
    ]
