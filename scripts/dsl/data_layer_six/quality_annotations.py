# SPDX-License-Identifier: Apache-2.0
"""Runtime-state ingress (SPQ-050): field names as syntax, annotation atoms through scoped imports.

Annotation atoms resolve through the imports of their scope chain (function scopes enclose; class
scopes do not), unless a binding of the same name in a nearer or equal scope shadows the import.
Scopes, imports, bindings and atoms are narrow rows; scope ownership is span containment.
"""
from .common import *

DOTTED=r'^[A-Za-z_]\w*(?:\s*\.\s*[A-Za-z_]\w*)*$'

def annotation_clauses(m,forbid,relation):
 from .quality import NOT_TESTS,PY_NAME,NAMED,TYPED,alt,member,keyword,modtext,str_lit
 entry='_check_serverplan_runtime_state_ingress'
 owners=m.SERVERPLAN_RUNTIME_OWNER_FIELD_NAMES;parts=m.SERVERPLAN_RUNTIME_SERVICE_NAME_PARTS
 forbid('ingress-owner-fields',entry,50,A(K('assignment'),F('left',names(owners)),TYPED,NOT_TESTS))
 forbid('ingress-service-fields',entry,50,A(K('assignment'),F('left',A(K('identifier'),RX(alt(parts)+'|bundle|runtime'))),TYPED,
  N(I(K('function_definition'),stop=O(K('function_definition'),K('class_definition')))),NOT_TESTS))
 forbid('ingress-unwrap-lists',entry,50,A(K('assignment'),F('left',A(K('identifier'),RX('UNWRAP|OWNER'))),N(TYPED),F('right',A(O(K('tuple'),K('list'),K('set')),H(str_lit(owners)))),NOT_TESTS))
 forbid('ingress-bundle-members',entry,50,A(member(['renewable_capability_bundle']),NOT_TESTS))
 for name in sorted(m.SERVERPLAN_RUNTIME_BUILDER_KWARGS):
  forbid('ingress-builder-'+name,entry,50,A(call(['runtime_builder_post_op']),keyword([name]),NOT_TESTS))

 # Resolved origins are built from file text, so a file without these tokens has no finding.
 tokens=modtext('Callable|'+alt(parts))
 annotation=lambda capture:I(A(K('assignment'),P(capture)),field='type')
 import_name=O(A(K('dotted_name'),P('IMPORTED')),A(K('aliased_import'),F('name',P('IMPORTED')),F('alias',P('ALIAS'))))
 quoted_atom=O(A(K('identifier'),N(I(K('attribute'),'neighbor')),{'capture':'ATOM'}),A(K('attribute'),N(I(K('attribute'),'neighbor')),{'capture':'ATOM'}))
 quoted=A(O(K('string'),K('concatenated_string')),annotation('ANN'),{'parse':{'grammar':'annotation','has':{**quoted_atom,'stopBy':'end'}}})
 parameters=lambda:A(K('parameters'),I(A(K('function_definition'),P('FN')),'neighbor'))
 generic=I(A(K('generic_type'),H(A({'nthChild':1},P('BASE')))),'neighbor')
 arg_project={'ann_start':'capture.ANN.startByte','base_start':'capture.BASE.startByte','base_end':'capture.BASE.endByte'}
 inputs={
  'fn_scopes':(A(K('function_definition'),F('name',P('NAME')),NOT_TESTS,tokens),{'name':'capture.NAME'}),
  'class_scopes':(A(K('class_definition'),F('name',P('NAME')),NOT_TESTS,tokens),{'name':'capture.NAME'}),
  'annotated':(A(K('assignment'),TYPED,F('left',A(K('identifier'),P('NAME'))),NOT_TESTS,tokens),{'name':'capture.NAME'}),
  'name_atoms':(A(K('identifier'),P('ATOM'),N(I(K('attribute'),'neighbor')),annotation('ANN'),NOT_TESTS,tokens),{'text':'capture.ATOM','ann_start':'capture.ANN.startByte'}),
  'member_atoms':(A(K('attribute'),P('ATOM'),F('attribute',P('ATTR')),O(N(I(K('attribute'),'neighbor')),N(RX(DOTTED))),annotation('ANN'),NOT_TESTS,tokens),{'text':'capture.ATOM','attr':'capture.ATTR','ann_start':'capture.ANN.startByte'}),
  'quoted_atoms':(A(quoted,NOT_TESTS,tokens),{'ann_start':'capture.ANN.startByte','atom':'capture.ATOM.text'},{'each':'ATOM'}),
  'any_args':(A(K('type_parameter'),generic,annotation('ANN'),H(A(NAMED,P('ARG'))),NOT_TESTS,tokens),arg_project,{'each':'ARG','anchor':'capture.ARG'}),
  'later_args':(A(K('type_parameter'),generic,annotation('ANN'),H(A({'nthChild':'n+2'},P('ARG'))),NOT_TESTS,tokens),arg_project,{'each':'ARG','anchor':'capture.ARG'}),
  'from_imports':(A(K('import_from_statement'),F('module_name',P('MODULE')),imported(import_name),NOT_TESTS,tokens),{'module':'capture.MODULE','name':'capture.IMPORTED','alias':'capture.ALIAS'},{'each':'IMPORTED'}),
  'direct_imports':(A(K('import_statement'),imported(import_name),NOT_TESTS,tokens),{'name':'capture.IMPORTED','alias':'capture.ALIAS'},{'each':'IMPORTED'}),
  # Bindings: names in assignment, loop, walrus and del targets, parameters, globals and nonlocals.
  'bound_names':(A(K('identifier'),P('NAME'),*PY_NAME,O(I(K('assignment'),field='left'),I(K('augmented_assignment'),field='left'),I(K('for_statement'),field='left'),
   I(K('for_in_clause'),field='left'),I(K('named_expression'),field='name'),I(K('delete_statement'))),NOT_TESTS,tokens),{'name':'capture.NAME'}),
  'plain_params':(A(K('identifier'),P('NAME'),I(parameters(),'neighbor'),NOT_TESTS,tokens),{'name':'capture.NAME','fn':'capture.FN.startByte'}),
  'typed_params':(A(K('identifier'),P('NAME'),{'nthChild':1},I(A(O(K('typed_parameter'),K('default_parameter'),K('typed_default_parameter')),I(parameters(),'neighbor')),'neighbor'),NOT_TESTS,tokens),{'name':'capture.NAME','fn':'capture.FN.startByte'}),
  'global_names':(A(K('identifier'),P('NAME'),I(K('global_statement'),'neighbor'),NOT_TESTS,tokens),{'name':'capture.NAME'}),
  'nonlocal_names':(A(K('identifier'),P('NAME'),I(K('nonlocal_statement'),'neighbor'),NOT_TESTS,tokens),{'name':'capture.NAME'}),
 }
 # A node's scope is its innermost enclosing function or class, else the module (-1).
 owner=lambda n:f"coalesce((SELECT s.id FROM scope_nodes s WHERE s.__path={n}.__path AND s.id<{n}.__start AND s.__end>={n}.__end ORDER BY s.id DESC LIMIT 1),-1)"
 first="rx_extract({0},'^[A-Za-z_]\\w*',0)"
 stages=[
 S('scope_nodes',"SELECT __path,__start AS id,__start,__end,'function_definition' AS kind,name FROM fn_scopes UNION ALL SELECT __path,__start,__start,__end,'class_definition',name FROM class_scopes",('__path','id')),
 S('annotations',"SELECT a.__path,a.__start,a.__end,"+owner('a')+" AS scope FROM annotated a",('__path','__start')),
 # A scope's enclosing scope is its innermost enclosing function, else the module.
 S('scopes',"SELECT s.__path,s.id,coalesce((SELECT f.id FROM scope_nodes f WHERE f.__path=s.__path AND f.kind='function_definition' AND f.id<s.id AND f.__end>=s.__end ORDER BY f.id DESC LIMIT 1),-1) AS enclosing FROM scope_nodes s UNION ALL SELECT DISTINCT __path,-1,NULL FROM annotations",('__path','id')),
 S('scope_chain',"WITH RECURSIVE chain(__path,scope,ancestor,depth) AS (SELECT DISTINCT __path,scope,scope,0 FROM annotations UNION ALL SELECT c.__path,c.scope,s.enclosing,c.depth+1 FROM chain c JOIN scopes s ON s.__path=c.__path AND s.id=c.ancestor WHERE s.enclosing IS NOT NULL) SELECT * FROM chain",('__path','scope','ancestor'),('__path','scope','depth')),
 S('scope_imports',"SELECT i.__path,"+owner('i')+" AS scope,coalesce(i.alias,i.name) AS name,i.module||'.'||i.name AS origin FROM from_imports i"
  " UNION SELECT i.__path,"+owner('i')+",coalesce(i.alias,rx_extract(i.name,'^[^.]+',0)),CASE WHEN i.alias IS NULL THEN rx_extract(i.name,'^[^.]+',0) ELSE i.name END FROM direct_imports i",('__path','scope','name')),
 S('scope_shadows',"SELECT b.__path,"+owner('b')+" AS scope,b.name FROM bound_names b"
  " UNION SELECT p.__path,p.fn,p.name FROM plain_params p UNION SELECT p.__path,p.fn,p.name FROM typed_params p"
  " UNION SELECT s.__path,"+owner('s')+",s.name FROM scope_nodes s"
  " UNION SELECT g.__path,-1,g.name FROM global_names g"
  " UNION SELECT n.__path,f.id,n.name FROM nonlocal_names n JOIN scope_nodes f ON f.__path=n.__path AND f.kind='function_definition' AND f.id<n.__start AND f.__end>=n.__end",('__path','scope','name')),
 S('annotation_atoms',"SELECT a.__path,a.__start AS annotation,a.scope,'name:'||n.__start AS atom_id,n.__start AS atom_start,n.__end AS atom_end,n.text AS atom FROM name_atoms n JOIN annotations a ON a.__path=n.__path AND a.__start=n.ann_start"
  " UNION SELECT a.__path,a.__start,a.scope,'member:'||n.__start||':'||n.__end,n.__start,n.__end,CASE WHEN rx_matches(n.text,"+q(DOTTED)+") THEN rx_replace(n.text,'\\s+','') ELSE n.attr END FROM member_atoms n JOIN annotations a ON a.__path=n.__path AND a.__start=n.ann_start"
  " UNION SELECT a.__path,a.__start,a.scope,'quoted:'||q.__start||':'||q.atom,q.__start,q.__end,q.atom FROM quoted_atoms q JOIN annotations a ON a.__path=q.__path AND a.__start=q.ann_start",('__path','annotation'),('__path','scope','atom')),
 S('atom_imports',"SELECT a.*,i.origin,c.depth,NOT EXISTS(SELECT 1 FROM scope_chain shadow_chain JOIN scope_shadows shadow ON shadow.__path=shadow_chain.__path AND shadow.scope=shadow_chain.ancestor AND shadow.name=i.name WHERE shadow_chain.__path=a.__path AND shadow_chain.scope=a.scope AND shadow_chain.depth<=c.depth) AS unshadowed"
  " FROM annotation_atoms a JOIN scope_chain c ON c.__path=a.__path AND c.scope=a.scope JOIN scope_imports i ON i.__path=c.__path AND i.scope=c.ancestor AND i.name="+first.format('a.atom')+
  " WHERE NOT EXISTS(SELECT 1 FROM scope_chain near JOIN scope_imports local ON local.__path=near.__path AND local.scope=near.ancestor AND local.name=i.name WHERE near.__path=a.__path AND near.scope=a.scope AND near.depth<c.depth)",('__path','annotation','atom_id')),
 S('resolved_atoms',"SELECT a.__path,a.annotation,a.atom_id,a.atom_start,a.atom_end,a.atom,i.origin||substr(a.atom,length("+first.format('a.atom')+")+1) AS origin,i.unshadowed AND (SELECT count(DISTINCT x.origin) FROM atom_imports x WHERE x.__path=i.__path AND x.annotation=i.annotation AND x.atom_id=i.atom_id)=1 AS trusted FROM annotation_atoms a JOIN atom_imports i ON i.__path=a.__path AND i.annotation=a.annotation AND i.atom_id=a.atom_id"
  " UNION SELECT a.__path,a.annotation,a.atom_id,a.atom_start,a.atom_end,a.atom,a.atom,0 FROM annotation_atoms a WHERE NOT EXISTS(SELECT 1 FROM atom_imports i WHERE i.__path=a.__path AND i.annotation=a.annotation AND i.atom_id=a.atom_id)",('__path','annotation','atom_id'),('__path','annotation','atom_start','atom_end')),
 # Literal[...] arguments and Annotated[...] metadata are not types.
 S('ignored_annotation_metadata',"SELECT DISTINCT a.__path,a.annotation,a.atom_id FROM resolved_atoms a JOIN (SELECT __path,__start,__end,ann_start,base_start,base_end,1 AS any_arg FROM any_args UNION ALL SELECT __path,__start,__end,ann_start,base_start,base_end,0 FROM later_args) g ON g.__path=a.__path AND g.ann_start=a.annotation AND a.atom_start>=g.__start AND a.atom_end<=g.__end"
  " JOIN resolved_atoms base ON base.__path=a.__path AND base.annotation=a.annotation AND base.atom_start=g.base_start AND base.atom_end=g.base_end AND base.trusted"
  " WHERE g.any_arg=1 AND base.origin IN ('typing.Literal','typing_extensions.Literal') OR g.any_arg=0 AND base.origin IN ('typing.Annotated','typing_extensions.Annotated')",('__path','annotation','atom_id')),
 ]
 kept="NOT EXISTS(SELECT 1 FROM ignored_annotation_metadata ignored WHERE ignored.__path=a.__path AND ignored.annotation=a.annotation AND ignored.atom_id=a.atom_id)"
 head="SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,'SPQ-050' AS issue,{} AS identity FROM annotations n WHERE EXISTS(SELECT 1 FROM resolved_atoms a WHERE a.__path=n.__path AND a.annotation=n.__start AND "
 rows=[head.format("'callable:'||n.__start")+"rx_matches(a.origin,'(?:^|\\.)Callable(?:\\.|$)') AND "+kept+')',
  head.format("'service:'||n.__start")+"NOT (a.trusted AND a.origin IN "+literals(m._CONNECTION_ID_SCALAR_EXPORTS)+") AND rx_matches(a.origin,"+q(alt(parts))+") AND "+kept+')']
 relation('runtime-state-annotations',entry,inputs,stages,rows,admit=(NOT_TESTS,RX('Callable|'+alt(parts))))
