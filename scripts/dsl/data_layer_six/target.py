# SPDX-License-Identifier: Apache-2.0
"""Authored source and inventory relations for the target-architecture port."""
from .common import *
import json,re

def target_clauses(m):
 clauses=[]
 def c(id,rule,code):clauses.append(C(id,rule,code,['scan_sources']))
 c('legacy-symbols',A(K('class_definition'),F('name',names(m._LEGACY_SYMBOLS))),'SPTA004')
 generic={'filePath':{'regex':r'^(?:impl/common/l2070_server_plan/|l2070_batching/)','notRegex':r'^impl/common/l2070_server_plan/codegen/'}}
 c('backend-names',A(O(K('class_definition'),K('function_definition')),generic,F('name',A(RX('(?i)'+'|'.join(m._BACKEND_PARTS)),RX(r'(?i)batch|coalesce|strategy|transport')))),'SPTA009')
 exec_path={'filePath':{'regex':r'^l2070_batching/executor\.py$'}}
 # Python AST annotation names include attribute receivers, not their attribute labels.
 word=A(K('identifier'),N(I(K('attribute'),'neighbor',field='attribute')),N(I(K('keyword_argument'),'neighbor',field='name')))
 param=A(O(K('typed_parameter'),K('typed_default_parameter')),N(H(O(K('list_splat_pattern'),K('dictionary_splat_pattern')))))
 c('executor-parameters',A(K('function_definition'),exec_path,F('parameters',H(A(param,F('type',O(A(word,RX('^ServerPlanStep$')),H(A(word,RX('^ServerPlanStep$')),stop='end')))),stop='end'))),'SPTA006')
 c('executor-imports',A(K('import_from_statement'),exec_path,F('module_name',RX(r'(?:\.steps\.|\.steps$)')),imported(A(O(A(K('dotted_name'),P('IMPORTED')),A(K('aliased_import'),F('name',P('IMPORTED')))),RX(r'^(?:[A-Za-z_]\w*)?(?:Step|Intent|Strategy)(?:\s+as\s+\w+)?$')))),'SPTA005')
 # Each alias produces its own finding; repeated captures enumerate imports.
 clauses[-1]['violation']['each']='IMPORTED'
 tags=O(call(['step_tags']),H(call(['step_tags']),stop='end'))
 c('tag-conditions',A(O(K('if_statement'),K('elif_clause'),K('conditional_expression')),F('condition',tags)),'SPTA007')
 c('tag-matches',A(K('match_statement'),O(F('subject',tags),H(A(K('case_clause'),H(A(K('if_clause'),tags),stop='end')),stop='end'))),'SPTA007')
 strategies={'filePath':{'regex':r'^impl/common/l2070_server_plan/strategies/'}}
 second=lambda rule:F('arguments',H(A(rule,{'nthChild':{'position':2,'ofRule':N(K('comment'))}})))
 c('strategy-getattr',A(call(['getattr']),strategies,second(A(O(K('string'),K('concatenated_string')),{'value':{'regex':r'^(?:_stmt|_grammar|grammar)$'}}))),'SPTA008')
 c('strategy-isinstance',A(call(['isinstance']),strategies,second(O(A(word,RX(r'^Sql|Grammar$|Stmt$')),H(A(word,RX(r'^Sql|Grammar$|Stmt$')),stop='end')))),'SPTA008')
 for path,prefixes in sorted(m._PROHIBITED_IMPORTS.items()):
  path_rule={'filePath':{'regex':'^'+re.escape(path)+'$'}}
  c('imports-'+path.replace('/','-')+'-from',A(K('import_from_statement'),path_rule,F('module_name',RX('^(?:'+'|'.join(re.escape(x) for x in prefixes)+')'))),'SPTA012')
  c('imports-'+path.replace('/','-')+'-direct',A(K('import_statement'),path_rule,imported(O(A(K('dotted_name'),P('IMPORT'),RX('^(?:'+'|'.join(re.escape(x) for x in prefixes)+')')),
    A(K('aliased_import'),F('name',A(P('IMPORT'),RX('^(?:'+'|'.join(re.escape(x) for x in prefixes)+')'))))))),'SPTA012')
  clauses[-1]['violation']['each']='IMPORT'
 receipt=A(K('attribute'),F('object',names(['receipt','execution_receipt'])))
 accounting=O(names(['driver_round_trips','query_rtts','round_trips']),A(K('attribute'),F('attribute',names(['driver_round_trips','query_rtts','round_trips']))))
 c('receipt-accounting',A(O(K('assignment'),K('augmented_assignment')),{'filePath':{'regex':r'^l9900_auto_api/'}},F('left',accounting),F('right',N(receipt))),'SPTA010')
 top_class=A(K('class_definition'),P('CLASS'),F('name',P('NAME')),N(I(O(K('class_definition'),K('function_definition')))))
 dc=A(K('decorator'),H(A(call(['dataclass']),F('arguments',A(H(A(K('keyword_argument'),F('name',{'pattern':'frozen'}),F('value',{'pattern':'True'}))),
   H(A(K('keyword_argument'),F('name',{'pattern':'slots'}),F('value',{'pattern':'True'}))))))))
 decorated=I(A(K('decorated_definition'),H(dc)),'neighbor')
 top_function=A(K('function_definition'),P('FN'),F('name',P('NAME')),N(I(O(K('function_definition'),K('class_definition')))),
  O(I(K('module'),'neighbor'),I(A(K('decorated_definition'),I(K('module'),'neighbor')),'neighbor')))
 method=A(K('function_definition'),P('METHOD'),F('name',P('METHOD_NAME')),
  O(I(A(K('block'),I(top_class,'neighbor')),'neighbor'),I(A(K('decorated_definition'),I(A(K('block'),I(top_class,'neighbor')),'neighbor')),'neighbor')))
 assignment=A(O(K('assignment'),K('augmented_assignment')),P('ASSIGN'),F('left',A(K('identifier'),P('NAME'))))
 top_assign=A(assignment,I(A(K('expression_statement'),I(K('module'),'neighbor')),'neighbor'))
 class_assign=A(K('assignment'),P('ASSIGN'),F('left',A(K('identifier'),P('FIELD'))),optional(F('type',P('TYPE'))),I(A(K('expression_statement'),I(A(K('block'),I(top_class,'neighbor')),'neighbor')),'neighbor'))
 inputs={
  'raw_classes':E(A(top_class,optional(A(decorated,P('FROZEN')))),{'name':'capture.NAME','frozen':'capture.FROZEN.startByte'}),
  'raw_functions':E(top_function,{'name':'capture.NAME'}),
  'raw_assignments':E(top_assign,{'name':'capture.NAME'}),
  'raw_members':E(class_assign,{'name':'capture.FIELD','owner':'capture.CLASS.startByte','type':'capture.TYPE'}),
  'raw_methods':E(method,{'name':'capture.METHOD_NAME','owner':'capture.CLASS.startByte'}),
  'raw_annotations':E(A(word,P('TOKEN'),I(A(K('type'),I(class_assign,'neighbor',field='type')))),{'name':'capture.TOKEN','owner':'capture.CLASS.startByte','field':'capture.FIELD'}),
  'raw_returns':E(A(word,P('TOKEN'),I(method,field='return_type')),{'name':'capture.TOKEN','owner':'capture.METHOD.startByte','class':'capture.CLASS.startByte','method':'capture.METHOD_NAME'}),
  'raw_parameters':E(A(word,P('TOKEN'),I(param,field='type'),I(method,K('function_definition'))),{'name':'capture.TOKEN','owner':'capture.METHOD.startByte','class':'capture.CLASS.startByte','method':'capture.METHOD_NAME'}),
  'raw_bases':E(A(word,P('TOKEN'),N(I(K('keyword_argument'))),I(A(K('argument_list'),I(top_class,'neighbor')))),{'name':'capture.TOKEN','owner':'capture.CLASS.startByte'}),
  'raw_union_names':E(A(word,P('TOKEN'),I(top_assign,field='right')),{'name':'capture.TOKEN','alias':'capture.NAME'}),
 }
 # Required inventories are source constants, not expected findings.
 def table(name,rows,fields):
  payload=q(json.dumps(sorted(rows),separators=(',',':')))
  return S(name,'SELECT '+','.join(f"json_extract(value,'$[{i}]') AS {field}" for i,field in enumerate(fields))+f' FROM json_each({payload})',*( (fields[0],fields[1]) ,))
 stages=[
  S('classes','SELECT * FROM (SELECT *,row_number() OVER(PARTITION BY __path,name ORDER BY __start DESC) AS selected FROM raw_classes) WHERE selected=1',('__path','name'),('__path','__start')),
  S('symbols','SELECT __path,name FROM classes UNION SELECT __path,name FROM raw_functions UNION SELECT __path,name FROM raw_assignments',('__path','name')),
  S('members','SELECT * FROM raw_members',('__path','owner','name')),
  S('methods','SELECT * FROM (SELECT *,row_number() OVER(PARTITION BY __path,owner,name ORDER BY __start DESC) AS selected FROM raw_methods) WHERE selected=1',('__path','owner','name')),
  S('annotations','SELECT * FROM raw_annotations',('__path','owner','field','name')),
  S('returns','SELECT * FROM raw_returns',('__path','owner','name')),
  S('parameters','SELECT * FROM raw_parameters',('__path','owner','name')),
  S('bases','SELECT * FROM raw_bases',('__path','owner','name')),
  S('union_names','SELECT * FROM raw_union_names',('__path','alias','name')),
  table('required_symbols',[(p,n) for p,ns in m._REQUIRED_SYMBOLS.items() for n in ns],['path','name']),
  table('required_fields',[(p,c,f) for (p,c),fs in m._REQUIRED_CLASS_FIELDS.items() for f in fs],['path','class','name']),
  table('required_immutable',list(m._REQUIRED_IMMUTABLE_CLASSES),['path','class']),
  table('required_enums',[(p,c,n) for (p,c),ns in m._REQUIRED_ENUM_MEMBERS.items() for n in ns],['path','class','name']),
  table('required_unions',[(p,c,n) for (p,c),ns in m._REQUIRED_UNION_MEMBERS.items() for n in ns],['path','alias','name']),
 ]
 query="""SELECT path,0 AS start_byte,0 AS end_byte,symbol,issue,requirement FROM (
 SELECT r.path,r.name AS symbol,'SPTA001' AS issue,'symbol' AS requirement FROM required_symbols r
 WHERE NOT EXISTS(SELECT 1 FROM symbols s WHERE s.__path=r.path AND s.name=r.name)
 UNION ALL SELECT DISTINCT r.path,r.class,'SPTA013','fields' FROM required_fields r JOIN classes c ON c.__path=r.path AND c.name=r.class
 WHERE NOT EXISTS(SELECT 1 FROM members f WHERE f.__path=r.path AND f.owner=c.__start AND f.name=r.name AND f.type IS NOT NULL)
 UNION ALL SELECT r.path,r.class,'SPTA013','immutable' FROM required_immutable r JOIN classes c ON c.__path=r.path AND c.name=r.class WHERE c.frozen IS NULL
 UNION ALL SELECT DISTINCT r.path,r.class,'SPTA013','enum' FROM required_enums r JOIN classes c ON c.__path=r.path AND c.name=r.class
 WHERE NOT EXISTS(SELECT 1 FROM bases b WHERE b.__path=r.path AND b.owner=c.__start AND b.name='StrEnum')
 OR NOT EXISTS(SELECT 1 FROM members f WHERE f.__path=r.path AND f.owner=c.__start AND f.name=r.name AND f.type IS NULL)
 UNION ALL SELECT DISTINCT r.path,r.alias,'SPTA013','union' FROM required_unions r JOIN symbols s ON s.__path=r.path AND s.name=r.alias
 WHERE NOT EXISTS(SELECT 1 FROM union_names n WHERE n.__path=r.path AND n.alias=r.alias AND n.name=r.name)
 UNION ALL SELECT 'l2070_batching/step.py','ServerPlanStep.planning_contract','SPTA002','planning-method' WHERE NOT EXISTS(
 SELECT 1 FROM classes c JOIN methods m ON m.__path=c.__path AND m.owner=c.__start JOIN returns r ON r.__path=m.__path AND r.owner=m.__start
 WHERE c.__path='l2070_batching/step.py' AND c.name='ServerPlanStep' AND m.name='planning_contract' AND r.name='StepPlanningContract')
 UNION ALL SELECT 'l2070_batching/physical_placement.py','ApplicationPlacementUnit.application','SPTA003','application-field' WHERE NOT EXISTS(
 SELECT 1 FROM classes c JOIN annotations a ON a.__path=c.__path AND a.owner=c.__start AND a.field='application'
 WHERE c.__path='l2070_batching/physical_placement.py' AND c.name='ApplicationPlacementUnit' AND a.name='RuntimeApplication')
 UNION ALL SELECT 'l2070_batching/physical_executor.py','BackendExecutor.execute_program','SPTA011','executor-method' WHERE NOT EXISTS(
 SELECT 1 FROM classes c JOIN methods m ON m.__path=c.__path AND m.owner=c.__start
 JOIN returns r ON r.__path=m.__path AND r.owner=m.__start JOIN parameters p ON p.__path=m.__path AND p.owner=m.__start
 WHERE c.__path='l2070_batching/physical_executor.py' AND c.name='BackendExecutor' AND m.name='execute_program' AND r.name='BackendExecutionReceipt' AND p.name='PhysicalTransportProgram')
 )"""
 locations=sorted(set(m._REQUIRED_SYMBOLS)|{'l2070_batching/step.py','l2070_batching/physical_placement.py','l2070_batching/physical_executor.py'})
 clauses.append({'id':'required-contracts','relational':{'inputs':inputs,'stages':stages,'locations':locations,
  'query':choice('requiredShape',query,query+' WHERE 0'),'key':['symbol','issue','requirement'],'code':'issue'},
  'violation':{'code':'missing_contract','message':'Missing target contract','anchor':'witness.span'}})
 return clauses
