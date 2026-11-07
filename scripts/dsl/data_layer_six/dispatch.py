# SPDX-License-Identifier: Apache-2.0
"""Typed dispatch and recursive executor-provenance relations."""
from .common import *
from .ast_relations import relational

def dispatch_clause(m):
 api=literals(m._BASE_SERVER_PLAN_EXECUTOR_API)
 base=q(m._BASE_SERVER_PLAN_FILE)
 stages=[
 S('attributes',"SELECT n.*,a.text AS attr,o.id AS object,o.text AS object_text FROM nodes n JOIN nodes a ON a.__path=n.__path AND a.id=n.attribute_kind||':'||n.attribute_start||':'||n.attribute_end JOIN nodes o ON o.__path=n.__path AND o.id=n.object_kind||':'||n.object_start||':'||n.object_end WHERE n.kind='attribute'",('__path','id')),
 S('calls',"SELECT n.*,f.id AS callee_id,f.kind AS callee_kind,coalesce(a.attr,f.text) AS callee,a.object AS receiver FROM nodes n JOIN nodes f ON f.__path=n.__path AND f.id=n.function_kind||':'||n.function_start||':'||n.function_end LEFT JOIN attributes a ON a.__path=f.__path AND a.id=f.id WHERE n.kind='call'",('__path','id')),
 S('arguments',"SELECT n.*,c.id AS call,row_number() OVER(PARTITION BY c.__path,c.id ORDER BY n.__start)-1 AS position FROM calls c JOIN nodes n ON n.__path=c.__path AND n.parent=c.arguments_kind||':'||c.arguments_start||':'||c.arguments_end WHERE n.kind NOT IN ('comment','keyword_argument','list_splat','dictionary_splat')",('__path','call','position')),
 S('descendants',"WITH RECURSIVE walk(__path,root,node) AS (SELECT __path,id,id FROM nodes UNION SELECT w.__path,w.root,e.dst FROM walk w JOIN edges e ON e.__path=w.__path AND e.src=w.node) SELECT * FROM walk",('__path','root','node')),
 S('executor_types',"""WITH RECURSIVE types(__path,name) AS (
 SELECT DISTINCT __path,'BackendExecutor' FROM nodes
 UNION SELECT n.__path,coalesce(rx_extract(n.text,'\\bas\\s+([A-Za-z_]\\w*)$',1),rx_extract(n.text,'([A-Za-z_]\\w*)$',1)) FROM nodes n
 WHERE n.kind IN ('aliased_import','dotted_name') AND rx_matches(n.text,'(?:^|\\b)[A-Za-z_]*BackendExecutor(?:\\s+as\\s+[A-Za-z_]\\w*)?$')
 UNION SELECT n.__path,l.text FROM nodes n JOIN nodes l ON l.__path=n.__path AND l.id=n.left_kind||':'||n.left_start||':'||n.left_end
 JOIN nodes r ON r.__path=n.__path AND r.id=n.right_kind||':'||n.right_start||':'||n.right_end JOIN types t ON t.__path=n.__path AND t.name=r.text WHERE n.kind='assignment' AND l.kind='identifier' AND r.kind='identifier'
 UNION SELECT n.__path,rx_extract(n.text,'^class\\s+([A-Za-z_]\\w*)',1) FROM nodes n JOIN types t ON t.__path=n.__path WHERE n.kind='class_definition' AND rx_matches(n.text,'^class\\s+\\w+\\([^\\n:]*\\b'||t.name||'\\b')
 ) SELECT * FROM types""",('__path','name')),
 S('annotation_edges',"""SELECT child.__path,child.id AS src,parent.id AS dst FROM nodes parent JOIN nodes child ON child.__path=parent.__path AND child.parent=parent.id
 WHERE parent.kind IN ('type','parenthesized_expression','union_type') OR parent.kind='binary_operator' AND instr(parent.text,'|')>0
 UNION SELECT arg.__path,arg.id,generic.id FROM nodes generic JOIN nodes head ON head.__path=generic.__path AND head.parent=generic.id AND head.ordinal=0 JOIN nodes arg ON arg.__path=generic.__path AND arg.parent=generic.id AND arg.ordinal>0
 WHERE generic.kind='subscript' AND rx_matches(head.text,'(?:^|\\.)(?:Annotated|Awaitable|Coroutine|Optional)$')
 UNION SELECT child.__path,child.id,generic.id FROM nodes generic JOIN nodes head ON head.__path=generic.__path AND head.parent=generic.id AND head.ordinal=0 JOIN nodes params ON params.__path=generic.__path AND params.parent=generic.id AND params.kind='type_parameter' JOIN nodes child ON child.__path=params.__path AND child.parent=params.id
 WHERE generic.kind='generic_type' AND rx_matches(head.text,'(?:^|\\.)(?:Annotated|Awaitable|Coroutine|Optional)$') AND (SELECT count(*) FROM nodes element WHERE element.__path=params.__path AND element.parent=params.id AND element.kind!='comment')=1""",('__path','src')),
 S('annotations',"""WITH RECURSIVE accepted(__path,id) AS (SELECT n.__path,n.id FROM nodes n WHERE
 (n.kind='identifier' AND (n.text IN (SELECT name FROM executor_types t WHERE t.__path=n.__path) OR n.text LIKE '%BackendExecutor'))
 OR (n.kind='attribute' AND (n.attribute_source IN (SELECT name FROM executor_types t WHERE t.__path=n.__path) OR n.attribute_source LIKE '%BackendExecutor'))
 OR (n.kind='string' AND rx_matches(n.text,'(?:^|[\\x22\\x27| ])(?:[A-Za-z_]\\w*\\.)*(?:[A-Za-z_]\\w*BackendExecutor|BackendExecutor)(?:[\\x22\\x27| ]|$)'))
 UNION SELECT a.__path,e.dst FROM accepted a JOIN annotation_edges e ON e.__path=a.__path AND e.src=a.id) SELECT * FROM accepted""",('__path','id')),
 S('factories',"SELECT DISTINCT f.__path,f.name FROM functions f JOIN annotations a ON a.__path=f.__path AND a.id=f.return_type_kind||':'||f.return_type_start||':'||f.return_type_end UNION SELECT DISTINCT __path,'_create_backend_executor' FROM nodes",('__path','name')),
 S('bindings',"""SELECT n.__path,n.id,n.__start,l.id AS target,r.id AS value,o.fn FROM nodes n JOIN nodes l ON l.__path=n.__path AND l.id=n.left_kind||':'||n.left_start||':'||n.left_end
 JOIN nodes r ON r.__path=n.__path AND r.id=n.right_kind||':'||n.right_start||':'||n.right_end LEFT JOIN ownership o ON o.__path=n.__path AND o.id=n.id
 WHERE n.kind IN ('assignment','for_statement','for_in_clause')
 UNION SELECT n.__path,n.id,n.__start,l.id,r.id,o.fn FROM nodes n JOIN nodes l ON l.__path=n.__path AND l.id=n.name_kind||':'||n.name_start||':'||n.name_end
 JOIN nodes r ON r.__path=n.__path AND r.id=n.value_kind||':'||n.value_start||':'||n.value_end LEFT JOIN ownership o ON o.__path=n.__path AND o.id=n.id WHERE n.kind='named_expression'""",('__path','target'),('__path','fn')),
 S('name_sites',"""SELECT n.__path,n.id,n.text,n.__start,o.fn,f.__start AS fn_start,f.__end AS fn_end,
 EXISTS(SELECT 1 FROM bindings store WHERE store.__path=n.__path AND store.target=n.id) AS is_store,
 (SELECT count(*) FROM bindings b JOIN nodes t ON t.__path=b.__path AND t.id=b.target WHERE b.__path=n.__path AND b.fn=o.fn AND t.text=n.text) AS binding_count,
 (SELECT b.target FROM bindings b JOIN nodes t ON t.__path=b.__path AND t.id=b.target WHERE b.__path=n.__path AND b.fn=o.fn AND t.text=n.text AND b.__start<n.__start ORDER BY b.__start DESC LIMIT 1) AS latest_target
 FROM nodes n JOIN ownership o ON o.__path=n.__path AND o.id=n.id JOIN functions f ON f.__path=o.__path AND f.id=o.fn WHERE n.kind='identifier'""",('__path','text','fn'),('__path','fn','text')),
 S('propagation',f"""SELECT __path,object AS src,id AS dst FROM attributes
 UNION SELECT n.__path,c.id,n.id FROM nodes n JOIN nodes c ON c.__path=n.__path AND c.parent=n.id
 WHERE n.kind IN ('parenthesized_expression','type','await','unary_operator','list','tuple','set','list_splat','expression_list','pattern_list')
 UNION SELECT n.__path,c.id,n.id FROM nodes n JOIN nodes c ON c.__path=n.__path AND c.parent=n.id
 WHERE n.kind IN ('boolean_operator','conditional_expression') AND (n.kind!='conditional_expression' OR c.ordinal IN (0,2))
 UNION SELECT n.__path,c.id,n.id FROM nodes n JOIN fields f ON f.__path=n.__path AND f.owner=n.id AND f.role='value' JOIN nodes c ON c.__path=n.__path AND c.id=f.child WHERE n.kind IN ('pair','named_expression')
 UNION SELECT n.__path,c.id,n.id FROM nodes n JOIN nodes c ON c.__path=n.__path AND c.parent=n.id WHERE n.kind='dictionary'
 UNION SELECT c.__path,c.receiver,c.id FROM calls c WHERE c.receiver IS NOT NULL AND c.callee NOT IN {api}
 UNION SELECT c.__path,a.id,c.id FROM calls c JOIN arguments a ON a.__path=c.__path AND a.call=c.id AND a.position=0 WHERE c.callee='getattr'
 UNION SELECT b.__path,b.value,b.target FROM bindings b JOIN nodes l ON l.__path=b.__path AND l.id=b.target WHERE l.kind NOT IN ('pattern_list','tuple_pattern','list_pattern')
 UNION SELECT b.__path,rv.id,lt.id FROM bindings b JOIN nodes l ON l.__path=b.__path AND l.id=b.target JOIN nodes r ON r.__path=b.__path AND r.id=b.value
 JOIN nodes lt ON lt.__path=l.__path AND lt.parent=l.id JOIN nodes rv ON rv.__path=r.__path AND rv.parent=r.id AND rv.ordinal=lt.ordinal
 WHERE l.kind IN ('pattern_list','tuple_pattern','list_pattern') AND r.kind IN ('expression_list','tuple','list')
 UNION SELECT b.__path,b.value,lt.id FROM bindings b JOIN nodes l ON l.__path=b.__path AND l.id=b.target JOIN nodes r ON r.__path=b.__path AND r.id=b.value JOIN nodes lt ON lt.__path=l.__path AND lt.parent=l.id
 WHERE l.kind IN ('pattern_list','tuple_pattern','list_pattern') AND r.kind NOT IN ('expression_list','tuple','list')
 UNION SELECT p.__path,p.id,n.id FROM name_sites p JOIN name_sites n ON n.__path=p.__path AND n.text=p.text
 WHERE NOT n.is_store AND p.fn_start<=n.fn_start AND p.fn_end>=n.fn_end AND (p.fn=n.fn OR n.binding_count=0)
 AND (n.binding_count<=1 OR p.id=n.latest_target)
 UNION SELECT m.__path,m.subject_kind||':'||m.subject_start||':'||m.subject_end,n.id FROM nodes m
 JOIN descendants d ON d.__path=m.__path AND d.root=m.id JOIN nodes n ON n.__path=d.__path AND n.id=d.node JOIN nodes p ON p.__path=n.__path AND p.id=n.parent
 WHERE m.kind='match_statement' AND n.kind='identifier' AND (p.kind IN ('as_pattern','splat_pattern') OR (p.kind='dotted_name' AND EXISTS(SELECT 1 FROM nodes cp WHERE cp.__path=p.__path AND cp.id=p.parent AND cp.kind='case_pattern' AND rx_matches(cp.text,'^[A-Za-z_]\\w*$'))))
 UNION SELECT p.__path,p.id,n.id FROM attributes p JOIN attributes n ON n.__path=p.__path AND n.attr=p.attr WHERE p.object_text IN ('self','cls') AND n.object_text IN ('self','cls')
 """,('__path','src'),('__path','dst')),
 ]
 # Selected literal carriers use their selected member, rather than their entire container.
 stages.extend([
 S('subscripts',"""SELECT n.*,c.id AS container,c.kind AS container_kind,c.text AS container_text FROM nodes n JOIN nodes c ON c.__path=n.__path AND c.parent=n.id AND c.ordinal=0 WHERE n.kind='subscript'""",('__path','id')),
 S('carrier_edges',"""SELECT s.__path,s.container AS src,s.id AS dst FROM subscripts s WHERE s.container_kind NOT IN ('list','tuple','dictionary')
 UNION SELECT s.__path,e.id,s.id FROM subscripts s JOIN nodes indexer ON indexer.__path=s.__path AND indexer.parent=s.id AND indexer.ordinal=1 JOIN nodes e ON e.__path=s.__path AND e.parent=s.container
 WHERE s.container_kind IN ('list','tuple') AND indexer.kind='integer' AND e.ordinal=CAST(indexer.text AS INTEGER)
 UNION SELECT s.__path,v.id,s.id FROM subscripts s JOIN nodes indexer ON indexer.__path=s.__path AND indexer.parent=s.id AND indexer.ordinal=1
 JOIN nodes pair ON pair.__path=s.__path AND pair.parent=s.container JOIN nodes k ON k.__path=pair.__path AND k.parent=pair.id AND k.ordinal=0 JOIN nodes v ON v.__path=pair.__path AND v.parent=pair.id AND v.ordinal=1
 WHERE s.container_kind='dictionary' AND k.text=indexer.text""",('__path','src')),
 S('owner_seeds',"""SELECT n.__path,n.id FROM nodes n JOIN nodes p ON p.__path=n.__path AND p.id=n.parent
 WHERE n.kind='identifier' AND ((p.kind='parameters' AND n.text IN ('backend','executor')) OR (p.kind IN ('typed_parameter','typed_default_parameter') AND n.ordinal=0 AND EXISTS(SELECT 1 FROM annotations a WHERE a.__path=p.__path AND a.id=p.type_kind||':'||p.type_start||':'||p.type_end)))
 UNION SELECT c.__path,c.id FROM calls c WHERE c.callee IN (SELECT name FROM factories f WHERE f.__path=c.__path) OR c.callee IN (SELECT name FROM executor_types t WHERE t.__path=c.__path) OR rx_matches(c.callee,'(?i)backend_executor')
 UNION SELECT c.__path,c.id FROM calls c JOIN arguments a ON a.__path=c.__path AND a.call=c.id AND a.position=0 JOIN annotations t ON t.__path=a.__path AND t.id=a.id WHERE c.callee='cast'
 UNION SELECT l.__path,l.id FROM nodes n JOIN nodes l ON l.__path=n.__path AND l.id=n.left_kind||':'||n.left_start||':'||n.left_end JOIN annotations a ON a.__path=n.__path AND a.id=n.type_kind||':'||n.type_start||':'||n.type_end WHERE n.kind='assignment'
 UNION SELECT a.__path,a.id FROM attributes a JOIN nodes n ON n.__path=a.__path AND n.kind='assignment' JOIN nodes l ON l.__path=n.__path AND l.id=n.left_kind||':'||n.left_start||':'||n.left_end JOIN annotations t ON t.__path=n.__path AND t.id=n.type_kind||':'||n.type_start||':'||n.type_end WHERE l.kind='identifier' AND l.text=a.attr AND a.object_text IN ('self','cls') AND EXISTS(SELECT 1 FROM nodes cls WHERE cls.__path=n.__path AND cls.kind='class_definition' AND rx_matches(cls.text,'^class BaseServerPlan\\b') AND cls.__start<n.__start AND cls.__end>n.__end)
 """,('__path','id')),
 S('owners',"""WITH RECURSIVE all_edges AS (SELECT * FROM propagation UNION SELECT * FROM carrier_edges), owners(__path,id) AS (
 SELECT * FROM owner_seeds UNION SELECT o.__path,e.dst FROM owners o JOIN all_edges e ON e.__path=o.__path AND e.src=o.id) SELECT * FROM owners""",('__path','id')),
 ])
 def emit(table,where,issue,identity):
  return f"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,{q(issue)} AS issue,{q(identity)}||n.id AS identity FROM {table} n WHERE {where}"
 owner=lambda expr:f"EXISTS(SELECT 1 FROM owners own WHERE own.__path=n.__path AND own.id={expr})"
 nonfactory="NOT EXISTS(SELECT 1 FROM ownership own JOIN functions f ON f.__path=own.__path AND f.id=own.fn WHERE own.__path=n.__path AND own.id=n.id AND f.name='_create_backend_executor')"
 basewhere=f'n.__path={base}'
 issues=[emit('attributes','n.attr IN '+literals(m._BACKEND_FAMILY_ATTRS),'SPD001','family'),
  emit('attributes',basewhere+f" AND (n.attr IN {literals(m._EXECUTOR_REACH_THROUGH_ATTRS-{'__dict__'})} OR ({owner('n.object')} AND n.attr NOT IN {api}))",'SPD005','attribute'),
  emit('subscripts',basewhere+' AND '+owner('n.id'),'SPD005','subscript'),
  emit('nodes',basewhere+" AND n.kind IN ('return_statement','yield') AND "+nonfactory+" AND EXISTS(SELECT 1 FROM nodes x JOIN owners own ON own.__path=x.__path AND own.id=x.id WHERE x.__path=n.__path AND x.parent=n.id AND x.kind='identifier')",'SPD005','escape'),
  emit('nodes',basewhere+" AND n.kind='assignment' AND n.left_kind IN ('attribute','subscript') AND "+owner("n.right_kind||':'||n.right_start||':'||n.right_end"),'SPD005','store'),
  emit('nodes',basewhere+" AND n.kind='comparison_operator' AND "+nonfactory+" AND EXISTS(SELECT 1 FROM nodes x JOIN owners own ON own.__path=x.__path AND own.id=x.id WHERE x.__path=n.__path AND x.parent=n.id)",'SPD005','compare'),
  emit('calls',basewhere+f" AND ((n.callee IN ('delattr','getattr','getattr_static','hasattr','setattr','vars','__getattribute__') AND EXISTS(SELECT 1 FROM arguments a JOIN owners own ON own.__path=a.__path AND own.id=a.id WHERE a.__path=n.__path AND a.call=n.id AND a.position=0) AND NOT EXISTS(SELECT 1 FROM arguments a WHERE a.__path=n.__path AND a.call=n.id AND a.position=1 AND trim(a.text,char(34)||char(39)) IN {api})) OR (n.callee_kind!='attribute' AND n.callee NOT IN {literals(m._BASE_SERVER_PLAN_EXECUTOR_CONSUMERS|m._REFLECTIVE_ACCESS_CALLS|{'cast'})} AND EXISTS(SELECT 1 FROM arguments a JOIN owners own ON own.__path=a.__path AND own.id=a.id WHERE a.__path=n.__path AND a.call=n.id)))",'SPD005','call')]
 query=' UNION ALL '.join(issues)
 clause=relational('executor-provenance',stages,query)
 for stage in clause['relational']['stages']:
  if stage['name']=='nodes':stage['indexes'].append(['__path','kind','text'])
  if stage['name']=='attributes':stage['indexes'].append(['__path','attr'])
 return clause

def dispatch_clauses(m):
 clause=dispatch_clause(m)
 # The provenance relation reads one file: check its path before any node capture, and admit the
 # relation only where that file's module is present, so other files build no SQL at all.
 base_file={'filePath':{'regex':'^'+re.escape(m._BASE_SERVER_PLAN_FILE)+'$'}}
 plan=clause['relational']
 # The base file declares BaseServerPlan; that literal is a per-file text prefilter for every input.
 base_text=I(A(K('module'),RX(r'class BaseServerPlan\b')))
 plan['inputs']['raw_nodes']['evidence']['query']['ast']['rule']['all'][:0]=[base_file,base_text]
 plan['inputs']['dispatch_gate']=E(A(base_file,K('module')),{'path':'file.path'})
 plan['requireNonemptyInput']='dispatch_gate'
 clauses=[clause]
 late=N({'filePath':{'regex':'^(?:'+'|'.join(re.escape(x) for x in sorted(m._LATE_BOUNDARY_FILES))+')$'}})
 def c(id,rule,code):clauses.append(C(id,A(late,rule),code,['scan_source']))
 # Attribute/owner diagnostics are already included by the relation.
 c('backend-family',A(K('attribute'),F('attribute',names(m._BACKEND_FAMILY_ATTRS)),N({'filePath':{'regex':'^'+re.escape(m._BASE_SERVER_PLAN_FILE)+'$'}})),'SPD001')
 backend=O(names(['backend_type']),A(K('attribute'),O(F('attribute',names(['backend_type'])),F('object',{'matches':'backend-source'}))),call(m._BACKEND_CALLS))
 backendliteral=O(A(K('attribute'),F('object',names(['BackendType']))),A(O(K('string'),K('concatenated_string')),{'value':{'regex':'(?i)^(?:'+'|'.join(sorted(m._BACKEND_LITERAL_VALUES))+')$'}}))
 c('backend-identity',A(K('comparison_operator'),O(backend,H(backend,stop='end')),O(backendliteral,H(backendliteral,stop='end'))),'SPD001')
 steptype=A(RX('^(?:'+'|'.join(re.escape(v) for v in sorted(m._STEP_TYPE_NAMES))+r'|[A-Za-z_]\w*(?:Intent|Step|Strategy))$'),N(names(m._NON_DISPATCH_STEP_TYPES)))
 typeleaf=O(A(K('identifier'),steptype),A(K('attribute'),F('attribute',steptype)))
 typeexpr=O(typeleaf,A(K('tuple'),H({'matches':'step-type'})),A(K('binary_operator'),RX(r'\|'),O(F('left',{'matches':'step-type'}),F('right',{'matches':'step-type'}))))
 arg2=lambda rule:F('arguments',H(A(rule,{'nthChild':{'position':2,'ofRule':N(K('comment'))}})))
 c('step-isinstance',A(call(['isinstance']),arg2({'matches':'step-type'})),'SPD002')
 c('step-type-compare',A(K('comparison_operator'),H(call(['type'])),H({'matches':'step-type'})),'SPD002')
 c('step-class-match',A(K('class_pattern'),H(O(A(K('dotted_name'),RX(r'(?:^|\.)'+r'(?:'+'|'.join(re.escape(v) for v in sorted(m._STEP_TYPE_NAMES))+r'|[A-Za-z_]\w*(?:Intent|Step|Strategy))$')),typeleaf))),'SPD002')
 typename=A(K('attribute'),F('attribute',names(['__name__'])),F('object',O(call(['type']),A(K('attribute'),F('attribute',names(['__class__']))))))
 c('step-name-compare',A(K('comparison_operator'),H(typename),H(A(O(K('string'),K('concatenated_string')),{'value':{'regex':'^(?:'+'|'.join(re.escape(v) for v in sorted(m._STEP_TYPE_NAMES))+r'|[A-Za-z_]\w*(?:Intent|Step|Strategy))$'}}),stop='end')),'SPD003')
 dialect=O(names(['dialect']),A(K('attribute'),F('attribute',names(['dialect']))),A(call(['getattr']),arg2(A(K('string'),{'value':{'regex':'^dialect$'}}))))
 c('dialect-compare',A(K('comparison_operator'),H(dialect),H(K('none'))),'SPD004')
 truthy=O(dialect,A(K('not_operator'),H(dialect)),A(call(['bool']),F('arguments',H(A(dialect,{'nthChild':{'position':1,'ofRule':N(K('comment'))}}))),N(F('arguments',H({'nthChild':{'position':2,'ofRule':N(K('comment'))}})))),A(K('boolean_operator'),H({'matches':'dialect-truth'})))
 condition=O(I(O(K('if_statement'),K('elif_clause'),K('conditional_expression'),K('while_statement')),'neighbor','condition'),I(K('assert_statement'),'neighbor'),I(K('if_clause'),'neighbor'))
 c('dialect-truth',A({'matches':'dialect-truth'},condition),'SPD004')
 return clauses,{'backend-source':backend,'step-type':typeexpr,'dialect-truth':truthy}
