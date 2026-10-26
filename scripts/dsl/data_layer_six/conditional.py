# SPDX-License-Identifier: Apache-2.0
"""Shared conditional architecture contracts used by ServerPlan quality.

The contracts are project facts: a subclass of the semantic conditional may live in any file, and
its bases resolve through each file's imports. The clause therefore stays global, but it reads
narrow candidate rows (classes, bases, imports, functions, and the few statement shapes the
contracts inspect) and joins them by span containment; it never copies the syntax tree.
"""
from .common import *

BRANCH_STEPS=r'then_plan\.steps|else_plan\.steps|conditional_branch_steps|then_steps|else_steps'

def conditional_clauses(m):
 semantic='l2070_batching.conditional_step.ConditionalStep'
 physical='impl.common.l2070_server_plan.steps.sql_conditional.SqlConditionalStep'
 compiler='impl.common.l2070_server_plan.steps._conditional_compile.compile_sql_conditional_step'
 executor='impl.common.l2070_server_plan.sql_backend_executor.SqlBackendExecutor'
 validation='require_sql_physical_conditional'
 entry='conditional_architecture_findings'
 enabled={'$option':'entryPoint','cases':{'all':choice('oracleAnalyzer',FALSE,TRUE),entry:TRUE},'default':FALSE}
 def row(rule,project,**kw):return E(A(enabled,rule),project,**kw)
 relevant=A(K('function_definition'),O(F('name',names([validation,'compile_sql_conditional_step'])),A({'filePath':{'regex':r'(?:^|/)l2070_batching/conditional_step\.py$'}},F('name',names(['compile'])))),F('name',P('NAME')))
 attr=lambda values:A(K('attribute'),F('attribute',names(values)))
 # Callable atoms: a bare identifier, never an attribute member, keyword, declaration name or parameter name.
 callable_atom=A(K('identifier'),RX('^Callable$'),N(I(K('attribute'),'neighbor','attribute')),
  N(I(O(K('keyword_argument'),K('function_definition'),K('class_definition')),'neighbor','name')),
  N(I(O(K('parameters'),K('lambda_parameters'),K('list_splat_pattern'),K('dictionary_splat_pattern')),'neighbor')))
 import_name=O(A(K('dotted_name'),P('IMPORTED')),A(K('aliased_import'),P('IMPORTED'),F('name',P('INAME')),F('alias',P('ALIAS'))))
 import_project={'kind':'capture.IMPORTED.kind','text':'capture.IMPORTED','iname':'capture.INAME','alias':'capture.ALIAS'}
 assign=O(K('assignment'),K('augmented_assignment'))
 branch_assign=A(assign,F('right',A(O(K('identifier'),K('attribute'),K('call')),RX(BRANCH_STEPS))),P('STMT'))
 branch_loop=A(K('for_statement'),F('right',RX(BRANCH_STEPS)),P('STMT'))
 backend=attr(['backend_type','is_sql_backend'])
 backend_in=A(RX(r'\b(?:backend_type|is_sql_backend)\b'),O(backend,H(backend,stop='end')))
 control=O(A(O(K('if_statement'),K('elif_clause')),F('condition',backend_in)),A(K('match_statement'),F('subject',backend_in)),
  A(K('conditional_expression'),H(A({'nthChild':2},backend_in))))
 inputs={
  'classes':row(A(K('class_definition'),F('name',P('NAME'))),{'name':'capture.NAME'}),
  # One row per class with bases; the base list is split in SQL (nested relations cost more than the split).
  'class_sups':row(A(K('class_definition'),F('superclasses',P('SUP'))),{'text':'capture.SUP'}),
  'from_names':row(A(K('import_from_statement'),F('module_name',P('MODULE')),imported(import_name)),{'module':'capture.MODULE',**import_project},each='IMPORTED',anchor='capture.IMPORTED'),
  'direct_names':row(A(K('import_statement'),imported(import_name)),import_project,each='IMPORTED',anchor='capture.IMPORTED'),
  'fns':row(A(K('function_definition'),F('name',P('NAME'))),{'name':'capture.NAME'}),
  'backend_fns':row(A(relevant,O(H(A(K('identifier'),RX('^BackendType$')),stop='end'),A(H(attr(['backend_type','is_sql_backend','dialect']),stop='end'),
   H(O(K('if_statement'),K('elif_clause'),K('conditional_expression'),K('match_statement')),stop='end')))),{'name':'capture.NAME'}),
  'raising_fns':row(A(relevant,H(A(K('raise_statement'),RX(r'^raise\s+TypeError(?:\s*\(|\s*$)')),stop='end')),{'name':'capture.NAME'}),
  'int_returns':row(A(fn('level'),H(A(K('return_statement'),P('R'),H(A(O(K('integer'),K('true'),K('false')),P('V')))),stop='end')),{'kind':'capture.V.kind','text':'capture.V'},each='R',anchor='capture.R'),
  'role_returns':row(A(fn('execution_role'),H(A(K('return_statement'),P('R'),H(RX(r'^StepExecutionRole\.SQL$'))),stop='end')),{'line':'capture.R.startLine'},each='R',anchor='capture.R'),
  'named_calls':row(A(K('call'),F('function',A(P('FN'),RX(r'compile_sql_conditional_step$|SqlConditionalStep\.from_semantic$|^step\.require_sql_physical_conditional$|_conditional_strategy$')))),{'full_callee':'capture.FN','line':'capture.FN.startLine'}),
  'self_stores':row(A(assign,F('left',A(K('attribute'),RX(r'^self\s*\.'),F('object',names(['self'])),F('attribute',P('ATTR')))),optional(F('right',P('RIGHT')))),{'attr':'capture.ATTR','expr_kind':'capture.RIGHT.kind','expr_text':'capture.RIGHT'}),
  'callable_params':row(A(K('function_definition'),P('FN'),F('parameters',A(RX(r'\bCallable\b'),H(A(O(K('typed_parameter'),K('typed_default_parameter')),H(A(K('identifier'),{'nthChild':1},P('PNAME'))),F('type',O(callable_atom,H(callable_atom,stop='end')))))))),{'name':'capture.PNAME','fn_start':'capture.FN.startByte'},each='PNAME'),
  'step_params':row(A(fn('compile_sql_conditional_step'),P('FN'),F('parameters',H(A(O(K('typed_parameter'),K('typed_default_parameter')),H(A(K('identifier'),{'nthChild':1},RX('^step$'))),F('type',P('ANN')))))),{'ann':'capture.ANN','fn_start':'capture.FN.startByte'}),
  'declared':row(A(callable_atom,P('ATOM'),I(A(K('assignment'),F('type',P('_')),N(I(K('function_definition'))),P('X')))),{'line':'capture.X.startLine','atom':'capture.ATOM.startByte'},anchor='capture.X'),
  'compile_calls':row(A(K('call'),F('function',A(K('attribute'),RX(r'\.\s*compile$'),F('attribute',names(['compile'])),F('object',P('OBJ'))))),{'object_text':'capture.OBJ'}),
  'branch_vars':row(A(O(branch_assign,branch_loop),F('left',O(A(K('identifier'),P('VAR')),H(A(K('identifier'),P('VAR')),stop='end')))),{'name':'capture.VAR','stmt_start':'capture.STMT.startByte','stmt_end':'capture.STMT.endByte'},each='VAR'),
  'controls':row(A(control,P('X')),{'line':'capture.X.startLine'}),
  # Blocks of the validation method, their statements and if arms: the inputs of the raise fold.
  'vnodes':row(A(O(K('block'),K('elif_clause'),K('else_clause'),K('case_clause'),A(O(K('raise_statement'),K('return_statement'),K('if_statement'),K('match_statement')),I(K('block'),'neighbor'))),I(fn(validation)),P('NODE'),I(P('PARENT'),'neighbor')),
   {'kind':'capture.NODE.kind','text':'capture.NODE','parent_start':'capture.PARENT.startByte','parent_end':'capture.PARENT.endByte','parent_kind':'capture.PARENT.kind'}),
 }
 inner_fn=lambda n:f"(SELECT f.__start FROM fn_spans f WHERE f.__path={n}.__path AND f.__start<{n}.__start AND f.__end>={n}.__end ORDER BY f.__start DESC LIMIT 1)"
 inner_cls=lambda n:f"(SELECT c.__start FROM class_spans c WHERE c.__path={n}.__path AND c.__start<{n}.__start AND c.__end>={n}.__end ORDER BY c.__start DESC LIMIT 1)"
 first=r"rx_extract({0},'^([\w]+)',1)"
 stages=[
 S('fn_spans','SELECT __path,__start,__end,name FROM fns',('__path','__start')),
 S('class_spans','SELECT __path,__start,__end,name FROM classes',('__path','__start'),('__path','name')),
 S('module_names',r"SELECT __path,CASE WHEN rx_matches(__path,'(?:^|/)(?:impl|l2070_batching|scripts)/') THEN replace(rx_extract(__path,'(?:^|/)((?:impl|l2070_batching|scripts)/.*)\.py$',1),'/','.') ELSE rx_extract(__path,'(?:^|/)([^/]+)\.py$',1) END AS qualified FROM (SELECT __path FROM classes UNION SELECT __path FROM fns)",('__path',)),
 S('aliases',r"""SELECT __path,__start,CASE WHEN kind='aliased_import' THEN alias ELSE text END AS local,trim(module,'.')||'.'||CASE WHEN kind='aliased_import' THEN iname ELSE text END AS origin FROM from_names
 UNION ALL SELECT __path,__start,CASE WHEN kind='aliased_import' THEN alias ELSE rx_extract(text,'^([\w]+)',1) END,CASE WHEN kind='aliased_import' THEN iname ELSE text END FROM direct_names""",('__path','local','__start')),
 S('cclasses',"SELECT c.__path,c.__start,c.__end,c.name,m.qualified||'.'||c.name AS qualified_name FROM class_spans c JOIN module_names m ON m.__path=c.__path WHERE c.__start=(SELECT max(o.__start) FROM class_spans o WHERE o.__path=c.__path AND o.name=c.name)",('__path','__start'),('qualified_name',)),
 # Top-level items of each base list; keyword arguments, splats and subscripts are not bases.
 S('base_scan',"""WITH RECURSIVE scan(__path,cls,t,i,depth,buf,item) AS (SELECT __path,__start,substr(text,2,length(text)-2),1,0,'',NULL FROM class_sups
 UNION ALL SELECT __path,cls,t,i+1,CASE WHEN substr(t,i,1) IN ('(','[','{') THEN depth+1 WHEN substr(t,i,1) IN (')',']','}') THEN depth-1 ELSE depth END,
 CASE WHEN i>length(t) OR substr(t,i,1)=',' AND depth=0 THEN '' ELSE buf||substr(t,i,1) END,CASE WHEN i>length(t) OR substr(t,i,1)=',' AND depth=0 THEN buf END FROM scan WHERE i<=length(t)+1)
 SELECT __path,cls AS __start,trim(item,char(32,9,10,13)) AS text FROM scan WHERE item IS NOT NULL""",('__path','__start')),
 S('bases',r"SELECT * FROM base_scan WHERE rx_matches(text,'^[A-Za-z_]\w*(?:\s*\.\s*[A-Za-z_]\w*)*$') OR rx_matches(text,'^[A-Za-z_][\w.\s]*\(') AND substr(text,-1)=')'",('__path','__start')),
 S('class_bases',"SELECT c.__path,c.__start AS cls,coalesce((SELECT a.origin FROM aliases a WHERE a.__path=c.__path AND a.local="+first.format('b.text')+" AND a.__start<c.__start ORDER BY a.__start DESC LIMIT 1),"+first.format('b.text')+r")||coalesce(rx_extract(b.text,'^[\w]+((?:\.[\w]+)*)',1),'') AS name FROM cclasses c JOIN bases b ON b.__path=c.__path AND b.__start=c.__start",('__path','cls','name')),
 S('derived',"WITH RECURSIVE derived(name) AS (SELECT "+q(semantic)+" UNION SELECT c.qualified_name FROM cclasses c JOIN class_bases b ON b.__path=c.__path AND b.cls=c.__start JOIN derived d ON d.name=b.name) SELECT * FROM derived",('name',)),
 S('fn_owner',"SELECT f.__path,f.__start,f.__end,f.name,"+inner_cls('f')+" AS cls FROM fn_spans f",('__path','cls','name')),
 S('cmethods',"SELECT f.__path,f.__start,f.__end,f.name,f.cls,c.qualified_name,c.qualified_name IN (SELECT name FROM derived) AS derived FROM fn_owner f JOIN cclasses c ON c.__path=f.__path AND c.__start=f.cls WHERE f.__start=(SELECT max(o.__start) FROM fn_owner o WHERE o.__path=f.__path AND o.cls=f.cls AND o.name=f.name)",('__path','cls','name'),('__path','__start')),
 S('cfunctions',"SELECT f.__path,f.__start,f.__end,f.name,m.qualified||'.'||f.name AS qualified_name FROM fn_owner f JOIN module_names m ON m.__path=f.__path WHERE f.cls IS NULL AND f.__start=(SELECT max(o.__start) FROM fn_owner o WHERE o.__path=f.__path AND o.cls IS NULL AND o.name=f.name)",('qualified_name',)),
 S('vtree',"SELECT __path,__start,__end,kind,text,kind||':'||__start||':'||__end AS id,parent_kind||':'||parent_start||':'||parent_end AS parent FROM vnodes",('__path','parent'),('__path','id')),
 S('contract_blocks',"SELECT b.*,row_number() OVER(PARTITION BY b.__path ORDER BY b.__end-b.__start,b.__start) AS sequence FROM vtree b JOIN cmethods m ON m.__path=b.__path AND m.name="+q(validation)+" AND m.__start="+inner_fn('b')+" WHERE b.kind='block'",('__path','id'),('__path','sequence')),
 ]
 # Fold inner blocks before outer blocks. The JSON map holds only booleans, keyed by CST identity.
 value="EXISTS(SELECT 1 FROM vtree stmt WHERE stmt.__path=b.__path AND stmt.parent=b.id AND stmt.__start<=coalesce((SELECT min(barrier.__start) FROM vtree barrier WHERE barrier.__path=b.__path AND barrier.parent=b.id AND barrier.kind IN ('raise_statement','return_statement')),b.__end) AND (stmt.kind='raise_statement' AND rx_matches(stmt.text,'^raise\\s+TypeError(?:\\s*\\(|\\s*$)') OR stmt.kind='if_statement' AND EXISTS(SELECT 1 FROM vtree alternative WHERE alternative.__path=stmt.__path AND alternative.parent=stmt.id AND alternative.kind='else_clause') AND NOT EXISTS(SELECT 1 FROM contract_blocks child WHERE child.__path=stmt.__path AND (child.parent=stmt.id OR EXISTS(SELECT 1 FROM vtree arm WHERE arm.__path=child.__path AND arm.id=child.parent AND arm.parent=stmt.id AND arm.kind IN ('elif_clause','else_clause'))) AND coalesce(json_extract(fold.state,'$.\"'||child.id||'\"'),0)!=1) OR stmt.kind='match_statement' AND EXISTS(SELECT 1 FROM vtree arm WHERE arm.__path=stmt.__path AND arm.kind='case_clause' AND arm.__start>stmt.__start AND arm.__end<=stmt.__end) AND NOT EXISTS(SELECT 1 FROM contract_blocks child JOIN vtree arm ON arm.__path=child.__path AND arm.id=child.parent AND arm.kind='case_clause' WHERE arm.__path=stmt.__path AND arm.__start>stmt.__start AND arm.__end<=stmt.__end AND coalesce(json_extract(fold.state,'$.\"'||child.id||'\"'),0)!=1)))"
 stages.extend([
 S('block_results',"WITH RECURSIVE fold(__path,sequence,state) AS (SELECT DISTINCT __path,0,'{}' FROM contract_blocks UNION ALL SELECT b.__path,b.sequence,substr(fold.state,1,length(fold.state)-1)||CASE WHEN length(fold.state)>2 THEN ',' ELSE '' END||json_quote(b.id)||':'||CAST(("+value+") AS TEXT)||'}' FROM fold JOIN contract_blocks b ON b.__path=fold.__path AND b.sequence=fold.sequence+1) SELECT __path,state FROM fold WHERE sequence=(SELECT max(b.sequence) FROM contract_blocks b WHERE b.__path=fold.__path)",('__path',)),
 S('always_raises',"SELECT m.__path,m.__start FROM cmethods m JOIN block_results r ON r.__path=m.__path JOIN vtree body ON body.__path=m.__path AND body.parent='function_definition:'||m.__start||':'||m.__end AND body.kind='block' WHERE m.name="+q(validation)+" AND json_extract(r.state,'$.\"'||body.id||'\"')=1",('__path','__start')),
 S('method_returns',"SELECT m.__path,m.cls,m.name,r.kind,r.text FROM cmethods m JOIN int_returns r ON r.__path=m.__path AND r.__start>=m.__start AND r.__start<=m.__end AND r.__end<=m.__end WHERE m.name='level'",('__path','cls')),
 S('owned_stores',"SELECT s.*,"+inner_fn('s')+" AS fn FROM self_stores s",('__path','fn')),
 S('owned_compiles',"SELECT c.*,"+inner_fn('c')+" AS fn FROM compile_calls c",('__path','fn')),
 S('owned_controls',"SELECT c.*,"+inner_fn('c')+" AS fn FROM controls c",('__path','fn')),
 S('owned_declared',"SELECT d.*,"+inner_cls('d')+" AS cls FROM declared d",('__path','cls')),
 ])
 rows=[]
 def add(detector,table,where,identity='n.__start',join=''):
  rows.append(f"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,'SPQ-029' AS issue,{identity} AS identity,{q(detector)} AS detector FROM {table} n {join} WHERE {where}")
 method=lambda name,condition='1':"EXISTS(SELECT 1 FROM cmethods method WHERE method.__path=n.__path AND method.cls=n.__start AND method.name="+q(name)+' AND ('+condition+'))'
 span=lambda table,alias,outer:f"EXISTS(SELECT 1 FROM {table} {alias} WHERE {alias}.__path={outer}.__path AND {alias}.__start>={outer}.__start AND {alias}.__end<={outer}.__end"
 calls=lambda name,pattern:method(name,span('named_calls','call','method')+" AND rx_matches(call.full_callee,"+q(pattern)+'))')
 level="SELECT 1 FROM method_returns r WHERE r.__path=n.__path AND r.cls=n.__start"
 add('semantic-compile','cclasses','n.qualified_name='+q(semantic)+' AND NOT '+method('compile','EXISTS(SELECT 1 FROM raising_fns r WHERE r.__path=method.__path AND r.__start=method.__start)'))
 add('semantic-level','cclasses','n.qualified_name='+q(semantic)+f" AND (NOT EXISTS({level}) OR EXISTS({level} AND (r.kind='false' OR r.kind='integer' AND CAST(r.text AS INTEGER)<=0)))")
 add('executor-hook','cclasses','n.qualified_name='+q(executor)+' AND NOT '+method('_lower_conditional_step'))
 add('physical-level','cclasses','n.qualified_name='+q(physical)+f" AND ((SELECT count(*) FROM method_returns r WHERE r.__path=n.__path AND r.cls=n.__start)!=1 OR NOT EXISTS({level} AND (r.kind='false' OR r.kind='integer' AND CAST(r.text AS INTEGER)=0)))")
 add('physical-role','cclasses','n.qualified_name='+q(physical)+' AND NOT '+method('execution_role',span('role_returns','r','method')+')'))
 add('physical-compile','cclasses','n.qualified_name='+q(physical)+' AND NOT '+calls('compile','compile_sql_conditional_step$'))
 owned="JOIN cmethods m ON m.__path=n.__path AND m.__start=n.fn AND m.derived"
 add('runtime-state','owned_stores',"m.name!='__init__'",join=owned)
 add('runtime-callback','owned_stores',"m.name='__init__' AND (n.expr_kind='lambda' OR n.expr_kind='identifier' AND EXISTS(SELECT 1 FROM callable_params p WHERE p.__path=n.__path AND p.fn_start=m.__start AND p.name=n.expr_text))",join=owned)
 add('declared-callback','owned_declared',"c.qualified_name IN (SELECT name FROM derived)",identity="n.__start||':'||n.atom",join="JOIN cclasses c ON c.__path=n.__path AND c.__start=n.cls")
 add('child-compilation','owned_compiles',"EXISTS(SELECT 1 FROM branch_vars b WHERE b.__path=n.__path AND b.name=n.object_text AND b.stmt_start>=m.__start AND b.stmt_end<=m.__end AND b.stmt_start<n.__start)",join=owned)
 add('backend-control','owned_controls','1',identity="m.__start||':'||n.line",join=owned)
 # Missing canonical owners use declared file-start locations, as in the upstream project analyzer.
 present="(EXISTS(SELECT 1 FROM cclasses c WHERE c.qualified_name IN "+literals([semantic,physical,executor])+") OR EXISTS(SELECT 1 FROM cfunctions f WHERE f.qualified_name="+q(compiler)+'))'
 raises='EXISTS(SELECT 1 FROM always_raises a WHERE a.__path=method.__path AND a.__start=method.__start)'
 selects='EXISTS(SELECT 1 FROM backend_fns color WHERE color.__path=method.__path AND color.__start=method.__start)'
 owners=[('semantic-contract',semantic,'NOT '+method(validation,raises)),
 ('physical-contract',physical,"n.qualified_name NOT IN (SELECT name FROM derived) OR NOT "+method(validation,'NOT '+raises+' AND NOT '+selects)),
 ('lowering-contract',executor,'NOT '+calls('_lower_conditional_step',r'SqlConditionalStep\.from_semantic$'))]
 for detector,owner,invalid in owners:
  add(detector,'cclasses','n.qualified_name='+q(owner)+' AND ('+invalid+') AND '+present)
  path=owner.rsplit('.',1)[0].replace('.','/')+'.py'
  rows.append("SELECT "+q(path)+" AS path,0 AS start_byte,0 AS end_byte,'SPQ-029' AS issue,'missing' AS identity,"+q(detector)+" AS detector WHERE "+present+" AND NOT EXISTS(SELECT 1 FROM cclasses c WHERE c.qualified_name="+q(owner)+')')
 annotation="coalesce((SELECT a.origin FROM aliases a WHERE a.__path=n.__path AND a.local="+first.format('p.ann')+" AND a.__start<n.__start ORDER BY a.__start DESC LIMIT 1),"+first.format('p.ann')+r")||coalesce(rx_extract(p.ann,'^[\w]+((?:\.[\w]+)*)$',1),'')"
 param="EXISTS(SELECT 1 FROM step_params p WHERE p.__path=n.__path AND p.fn_start=n.__start AND "+annotation+'='+q(physical)+')'
 valid=param+" AND NOT EXISTS(SELECT 1 FROM backend_fns c WHERE c.__path=n.__path AND c.__start=n.__start) AND "+span('named_calls','call','n')+" AND call.full_callee='step."+validation+"' AND NOT "+span('named_calls','strategy','n')+" AND rx_matches(strategy.full_callee,'_conditional_strategy$') AND strategy.line<=call.line))"
 add('compiler-contract','cfunctions','n.qualified_name='+q(compiler)+' AND NOT ('+valid+') AND '+present)
 rows.append("SELECT 'impl/common/l2070_server_plan/steps/_conditional_compile.py' AS path,0 AS start_byte,0 AS end_byte,'SPQ-029' AS issue,'missing' AS identity,'compiler-contract' AS detector WHERE "+present+" AND NOT EXISTS(SELECT 1 FROM cfunctions f WHERE f.qualified_name="+q(compiler)+')')
 locations=[name.rsplit('.',1)[0].replace('.','/')+'.py' for name in [semantic,physical,executor,compiler]]
 return [{'id':'conditional-architecture','relational':{'inputs':inputs,'stages':stages,'query':' UNION ALL '.join(rows),'key':['detector','issue','identity'],'code':'issue','locations':locations},
  'violation':{'code':'architecture','message':'Architecture contract violation','anchor':'witness.span'}}]
