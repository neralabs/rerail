# SPDX-License-Identifier: Apache-2.0
"""Constructor contracts and propagation of intent shape into lower().

Every node-local fact is an AST condition on a narrow candidate row: step-class membership of a
method (a non-keyword base naming ServerPlanStep/IntentStep), the NotImplementedError stub body,
parameter names and annotation atoms (union, ServerRef, raw builtins), dataclass decorators and
their annotated body fields. The relational part keeps only the joins of independently matched
rows: dataclass names referenced by annotation heads (a fixpoint over bad configs), constructor
parameters copied into self attributes, and the value flow of those attributes through lower()
assignments into branch conditions. "Descendant of" is span containment; there is no CST copy.
"""
from .common import *

def signature_clauses(m):
 entries=['scan_source','scan_tree','scan_project_sources']
 def row(rule,project):
  return E(A(*rule['all'],gate(entries)),project)
 named=N(K('comment'))
 base=lambda pattern:H(A(N(K('keyword_argument')),named,RX(pattern)))
 step_cls=lambda cap:A(K('class_definition'),P(cap),F('superclasses',base('(?:ServerPlanStep|IntentStep)')),optional(F('superclasses',A(base('IntentStep'),P(cap+'I')))))
 # A method: a function directly in a class body, or decorated in a class body.
 def method(cap,*extra):
  body=lambda:I(A(K('block'),I(step_cls(cap+'C'),'neighbor','body')),'neighbor')
  return A(K('function_definition'),P(cap),F('name',P(cap+'N')),*extra,O(body(),I(A(K('decorated_definition'),body()),'neighbor')))
 # The body is `raise NotImplementedError(...)` after only docstrings/literals, assignments, del and pass.
 literal=O(*(K(k) for k in ['assignment','string','concatenated_string','integer','float','none','true','false']))
 stub=F('body',H(A(K('raise_statement'),RX(r'^raise\s+NotImplementedError\s*\('),
  N({'precedes':{**named,'stopBy':'end'}}),
  N({'follows':{**A(named,N(O(K('expression_statement'),K('delete_statement'),K('pass_statement')))),'stopBy':'end'}}),
  N({'follows':{**A(K('expression_statement'),N(H(literal))),'stopBy':'end'}}))))
 union=O(A(O(K('union_type'),K('binary_operator')),RX(r'\|')),names(['Union','Optional']))
 atoms=lambda:[optional(F('type',P('T'))),optional(F('type',H(A(union,P('UN')),stop='end'))),optional(F('type',H(A(K('identifier'),RX('ServerRef$'),P('SR')),stop='end'))),
  optional(F('type',H(A(names(['str','int','float','bool','bytes']),P('RB')),stop='end'))),optional(F('type',H(A(names(['str']),P('HS')),stop='end')))]
 annotation={'ann':'capture.T','un':'capture.UN.startByte','sr':'capture.SR.startByte','rb':'capture.RB.startByte','hs':'capture.HS.startByte'}
 dataclass_dec=A(K('decorated_definition'),RX(r'(?m)^\s*@(?:[\w.]*\.)?dataclass\b'))
 # Inside a lower() or __init__ method of a step class (the nearest such method).
 in_body=I(method('M',F('name',names(['__init__','lower']))))
 body={'m_start':'capture.M.startByte','m_end':'capture.M.endByte','mname':'capture.MN','mc':'capture.MC.startByte'}
 inputs={
  # Parameters of top-level step-class methods (not nested in a function), with the stub flag.
  'sig_params':row(A(O(K('identifier'),K('typed_parameter'),K('default_parameter'),K('typed_default_parameter')),P('PARAM'),
    I(A(K('parameters'),I(method('M',N(I(K('function_definition'))),optional(A(stub,P('STUB')))),'neighbor','parameters')),'neighbor'),
    O(A(K('identifier'),P('PN')),H(A(K('identifier'),{'nthChild':{'position':1,'ofRule':named}},P('PN')))),*atoms()),
   {'kind':'capture.PARAM.kind','name':'capture.PN','m_start':'capture.M.startByte','m_end':'capture.M.endByte','method':'capture.MN','cls':'capture.MC.startByte',
    'intent':'capture.MCI.startByte','stub':'capture.STUB.startByte',**annotation}),
  'sig_dataclasses':row(A(K('class_definition'),F('name',P('NAME')),I(A(dataclass_dec,optional(A(RX(r'(?s)@(?:[\w.]*\.)?dataclass\s*\([^)]*\bfrozen\s*=\s*True'),P('FZ')))),'neighbor')),
   {'name':'capture.NAME','fz':'capture.FZ.startByte'}),
  # Annotated assignments directly in a dataclass body.
  'sig_fields':row(A(K('assignment'),F('type',K('type')),I(A(K('expression_statement'),I(A(K('block'),I(A(K('class_definition'),P('DC'),I(dataclass_dec,'neighbor')),'neighbor','body')),'neighbor')),'neighbor'),*atoms()),
   {'dc':'capture.DC.startByte',**annotation}),
  'sig_idents':row(A(K('identifier'),P('ID'),in_body),{'text':'capture.ID',**body}),
  'sig_assigns':row(A(K('assignment'),in_body,I(A(K('function_definition'),P('FN'))),F('right',P('R')),
    optional(F('left',A(K('identifier'),P('LT')))),optional(F('left',A(K('attribute'),F('object',names(['self'])),F('attribute',P('X')))))),
   {'fn_start':'capture.FN.startByte','fn_end':'capture.FN.endByte','r_start':'capture.R.startByte','r_end':'capture.R.endByte','lt':'capture.LT','x':'capture.X',**body}),
  'sig_selfattrs':row(A(K('attribute'),F('object',names(['self'])),F('attribute',P('X')),in_body),{'x':'capture.X',**body}),
  # Branches of lower(): the condition (if/elif/while), the test of a conditional expression
  # (its second child), or the operator itself.
  'sig_branches':row(A(O(*(K(k) for k in ['if_statement','elif_clause','while_statement','conditional_expression','not_operator','boolean_operator'])),P('B'),in_body,
    optional(F('condition',P('C'))),optional(A(K('conditional_expression'),H(A({'nthChild':{'position':2,'ofRule':named}},named,P('CE')))))),
   {'kind':'capture.B.kind','line':'capture.B.startLine','c_start':'capture.C.startByte','c_end':'capture.C.endByte','ce_start':'capture.CE.startByte','ce_end':'capture.CE.endByte',**body}),
 }
 # One merged input (each input is a corpus pass of its own); a view per role restores its table.
 from .state_flow import merge_roles
 roles=list(inputs)
 views,_=merge_roles(inputs,roles,entries,materialize=roles)
 names_re='|'.join(sorted(m.STRUCTURAL_CONSTANT_TYPE_NAMES))
 head="coalesce(rx_extract(ann,'^\\s*(?:[\\w.]+\\.)?([A-Za-z_]\\w*)',1),'')"
 basic=("CASE WHEN ann IS NULL THEN NULL ELSE sr IS NOT NULL AND un IS NULL OR un IS NULL AND (head='type' AND instr(ann,'[')>0 OR rx_matches(ann,"+q(r'^\s*(?:[A-Za-z_]\w*\.)*(?:'+names_re+r')\s*$')+") OR rx_matches(ann,"
  +q(r'^\s*(?:(?:[A-Za-z_]\w*\.)*frozenset\s*\[\s*)+(?:[A-Za-z_]\w*\.)*(?:'+names_re+r')\s*(?:\]\s*)+$')+")) END")
 contract=lambda table:f"SELECT *,{basic} AS basic FROM (SELECT *,CASE WHEN ann IS NULL THEN NULL ELSE {head} END AS head FROM {table})"
 stages=[*views,
 S('dataclasses',"SELECT __path,__start,__end,name,fz IS NOT NULL AND rx_matches(name,'(?:Config|Payload)$') AS static FROM sig_dataclasses",('__path','name'),('__path','__start')),
 S('config_fields',"SELECT f.__path,c.name AS cls,f.head,f.basic FROM ("+contract('sig_fields')+") f JOIN dataclasses c ON c.__path=f.__path AND c.__start=f.dc",('__path','cls')),
 S('bad_configs',"WITH RECURSIVE bad(__path,name) AS (SELECT c.__path,c.name FROM dataclasses c WHERE NOT c.static AND (NOT EXISTS(SELECT 1 FROM config_fields f WHERE f.__path=c.__path AND f.cls=c.name) OR EXISTS(SELECT 1 FROM config_fields f WHERE f.__path=c.__path AND f.cls=c.name AND NOT f.basic AND NOT EXISTS(SELECT 1 FROM dataclasses other WHERE other.__path=f.__path AND other.name=f.head))) "
  "UNION SELECT c.__path,c.name FROM dataclasses c JOIN config_fields f ON f.__path=c.__path AND f.cls=c.name JOIN bad b ON b.__path=f.__path AND b.name=f.head WHERE NOT c.static AND NOT f.basic) SELECT * FROM bad",('__path','name')),
 S('signature_parameters',"SELECT p.__path,p.__start,p.__end,p.kind,p.name,p.m_start,p.m_end,p.method,p.cls,p.intent IS NOT NULL AS intent,p.head,p.un IS NOT NULL AS union_type,p.sr IS NOT NULL AS server_ref,p.rb IS NOT NULL AS raw_builtin,p.hs IS NOT NULL AS has_str,"
  "coalesce(p.basic,0) OR EXISTS(SELECT 1 FROM dataclasses c WHERE c.__path=p.__path AND c.name=p.head AND NOT EXISTS(SELECT 1 FROM bad_configs b WHERE b.__path=c.__path AND b.name=c.name)) AS allowed FROM ("+contract('sig_params')+") p WHERE p.stub IS NULL",('__path','m_start')),
 S('shape_attributes',"SELECT DISTINCT a.__path,p.cls,a.x AS name FROM signature_parameters p JOIN sig_assigns a ON a.__path=p.__path AND a.fn_start=p.m_start AND a.fn_end=p.m_end AND a.x IS NOT NULL "
  "WHERE p.intent AND p.method='__init__' AND p.name NOT IN ('self','ref_id') AND NOT p.allowed AND EXISTS(SELECT 1 FROM sig_idents v WHERE v.__path=a.__path AND v.text=p.name AND v.__start>=a.r_start AND v.__end<=a.r_end)",('__path','cls','name')),
 # Shape members of each lower(): self attributes copied from constructor parameters, and the
 # names assigned from a value containing a member. A node is a shape value when it contains one.
 S('shape_values',"WITH RECURSIVE shape(__path,m_start,m_end,__start,__end) AS (SELECT s.__path,s.m_start,s.m_end,s.__start,s.__end FROM sig_selfattrs s JOIN shape_attributes x ON x.__path=s.__path AND x.cls=s.mc AND x.name=s.x WHERE s.mname='lower' "
  "UNION SELECT i.__path,sh.m_start,sh.m_end,i.__start,i.__end FROM shape sh JOIN sig_assigns a ON a.__path=sh.__path AND a.fn_start=sh.m_start AND a.fn_end=sh.m_end AND a.r_start<=sh.__start AND a.r_end>=sh.__end AND a.lt IS NOT NULL "
  "JOIN sig_idents i ON i.__path=a.__path AND i.text=a.lt AND i.__start>=sh.m_start AND i.__end<=sh.m_end) SELECT * FROM shape",('__path','m_start','__start')),
 S('shape_branches',"SELECT b.__path,b.__start,b.__end,b.line,(b.m_start*16777216+b.m_end) AS fn FROM (SELECT *,CASE WHEN kind='conditional_expression' THEN ce_start WHEN kind IN ('if_statement','elif_clause','while_statement') THEN c_start ELSE __start END AS t_start,"
  "CASE WHEN kind='conditional_expression' THEN ce_end WHEN kind IN ('if_statement','elif_clause','while_statement') THEN c_end ELSE __end END AS t_end FROM sig_branches WHERE mname='lower') b "
  "WHERE EXISTS(SELECT 1 FROM shape_values s WHERE s.__path=b.__path AND s.m_start=b.m_start AND s.m_end=b.m_end AND s.__start>=b.t_start AND s.__end<=b.t_end)"),
 ]
 rows=[]
 def add(code,where,table='signature_parameters',identity="n.__start||':'||n.__end"):
  rows.append(f"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,{q(code)} AS issue,{identity} AS identity,'signature' AS detector FROM {table} n WHERE {where}")
 add('SPT-015',"n.name NOT IN ('self','ref_id') AND n.raw_builtin AND NOT (n.name='config' AND EXISTS(SELECT 1 FROM dataclasses c WHERE c.__path=n.__path AND c.name=n.head))")
 add('SPT-016',"n.intent AND n.method='__init__' AND n.name NOT IN ('self','ref_id') AND n.kind IN ('default_parameter','typed_default_parameter')")
 add('SPT-017',"n.intent AND n.method='__init__' AND n.name NOT IN ('self','ref_id') AND NOT n.allowed")
 add('SPT-013',"n.method='__init__' AND n.name!='self' AND n.server_ref AND n.union_type")
 add('SPT-014',"n.method='__init__' AND n.name!='self' AND instr(lower(n.name),'table')>0 AND instr(lower(n.name),'alias')=0 AND n.has_str")
 # A branch's line is the event identity and anchor, rather than its method.
 add('SPT-018','1','shape_branches',"n.fn||':'||n.line")
 full=' UNION ALL '.join(rows)
 query={'$option':'entryPoint','cases':{e:full for e in ['all',*entries]},'default':'SELECT * FROM ('+full+') WHERE 0'}
 return [{'id':'state-signatures','relational':{'inputs':inputs,'stages':stages,'query':query,'key':['detector','issue','identity'],'code':'issue','partitionBy':'file'},
  'violation':{'code':'architecture','message':'Architecture contract violation','anchor':'witness.span'}}]
