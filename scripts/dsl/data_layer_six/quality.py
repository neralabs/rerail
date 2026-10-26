# SPDX-License-Identifier: Apache-2.0
"""ServerPlan quality diagnostics as syntax clauses and narrow relations.

A detector condition local to one node and its neighbourhood is a forbid clause. Only detectors
that join independently matched sets (sync/async pairs by name, the pre-op fixpoint over function
names, module string constants, projection repairs by loop variable, and annotation import
resolution) are relational. They read narrow candidate rows and join them by span containment;
no clause copies the syntax tree.
"""
from .common import *

LITERALS=('string','concatenated_string','integer','float','true','false','none')
NOT_TESTS={'filePath':{'notRegex':r'(?:^|/)tests(?:/|$)'}}

def exact(values):return '^(?:'+'|'.join(re.escape(v) for v in sorted(values))+')$'
def alt(values):return '|'.join(re.escape(v) for v in sorted(values))
# An identifier read as a Python name: not a declaration, keyword or member label, not in an import.
PY_NAME=(N(I(O(K('function_definition'),K('class_definition'),K('keyword_argument')),'neighbor','name')),N(I(K('attribute'),'neighbor','attribute')))
def py_name(values):
 return A(K('identifier'),RX(exact(values)),*PY_NAME,N(I(O(K('import_statement'),K('import_from_statement')))))
def member(values):return A(K('attribute'),F('attribute',names(values)))
def keyword(values,value=None):return F('arguments',H(A(K('keyword_argument'),F('name',names(values)),*([F('value',value)] if value else []))))
def base(name):return F('superclasses',H(A(N(K('keyword_argument')),RX(r'(?:^|\.)'+re.escape(name)+r'(?:\[.*\])?$'))))
def class_field(cls):
 """An assignment statement directly in a class body."""
 return I(A(K('expression_statement'),I(A(K('block'),I(cls,'neighbor','body')),'neighbor')),'neighbor')
def both(*items):
 items=[item for item in items if item!=TRUE]
 return TRUE if not items else items[0] if len(items)==1 else A(*items)

# Statement shapes. A body statement is any named child but a comment or a literal expression
# statement (docstrings); positional arguments exclude keywords and splats. `nthChild` counts named
# children only, so `NAMED` is "any named child".
LIT=O(*(K(k) for k in LITERALS))
STMT=A(N(K('comment')),N(A(K('expression_statement'),H(LIT))))
NAMED={'nthChild':'n+1'}
POSARG=A(N(K('comment')),N(K('keyword_argument')),N(K('list_splat')),N(K('dictionary_splat')))
SCOPE_STOP=O(K('function_definition'),K('class_definition'),K('lambda'))
NOT_ASYNC=N(RX('^async'))
def nth_stmt(n):return {'nthChild':{'position':n,'ofRule':STMT}}
def arg(n,rule):return F('arguments',H(A(rule,{'nthChild':{'position':n+1,'ofRule':POSARG}})))
def fp(regex):return {'filePath':{'regex':regex}}
def nfp(regex):return {'filePath':{'notRegex':regex}}
def modtext(*patterns):return I(A(K('module'),*(RX(p) for p in patterns)))
def str_lit(values):
 """A plain string literal whose decoded value is one of `values` (quoted source text, no escapes)."""
 a='(?:'+alt(values)+')'
 return A(K('string'),RX("^[rRuU]*(?:'''"+a+"'''|\"\"\""+a+"\"\"\"|'"+a+"'|\""+a+"\")$"))
def self_or(rule):return O(rule,H(rule,stop='end'))
def body_of(cls):return A(K('block'),I(cls,'neighbor','body'))
def in_class(cls=K('class_definition')):
 body=body_of(cls);return O(I(body,'neighbor'),I(A(K('decorated_definition'),I(body,'neighbor')),'neighbor'))
def method(name=None,cls=K('class_definition'),*extra):
 named=[F('name',names([name] if isinstance(name,str) else name))] if name else []
 return A(K('function_definition'),*named,*extra,in_class(cls))
def has_method(rule):return F('body',H(O(rule,A(K('decorated_definition'),H(rule)))))
MODULE_FN=A(K('function_definition'),O(I(K('module'),'neighbor'),I(A(K('decorated_definition'),I(K('module'),'neighbor')),'neighbor')))
def cls_named(values):return A(K('class_definition'),F('name',names(values)))
def cls_base(name):return A(K('class_definition'),base(name))
def calls_in(values,stop='end'):return H(call(values),stop=stop)
def import_named(values):return O(A(K('dotted_name'),RX(exact(values))),A(K('aliased_import'),F('name',RX(exact(values)))))
# A class's level is its `level` method's integer return (the detectors compare it with 4).
LEVEL4=has_method(A(fn('level'),H(A(K('return_statement'),H(A(K('integer'),RX('^4$')))),stop='end'),N(H(A(K('return_statement'),H(A(K('integer'),N(RX('^4$'))))),stop='end'))))
RETURNS_TRUE=F('body',A(H(A(K('return_statement'),H(K('true')),nth_stmt(1))),N(H(A(STMT,nth_stmt(2))))))
TYPED=F('type',P('_'))

def conjuncts(rule):
 if isinstance(rule,dict) and list(rule)==['all']:return [c for item in rule['all'] for c in conjuncts(item)]
 return [rule]
def gated(rule,gate):
 """The rule's root kind first, then the entry and file gate, then the rest of the rule."""
 items=conjuncts(rule)
 if items and list(items[0])==['kind']:return A(items[0],gate,*items[1:])
 return A(gate,*items)

def row(code,table,where='1',identity='n.__start'):
 return f"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,{q(code)} AS issue,{identity} AS identity FROM {table if ' ' in table else table+' n'} WHERE {where}"

def quality_clauses(m):
 def rel(path):return path.relative_to(m.ROOT).as_posix() if path.is_relative_to(m.ROOT) else path.as_posix()
 def exact_paths(paths):return '^(?:'+'|'.join(re.escape(v) for v in sorted({rel(p) for p in paths}))+')$'
 def at(path):return fp(exact_paths([path]))
 def not_at(path):return nfp(exact_paths([path]))
 def under(*paths):return fp('^(?:'+'|'.join(re.escape(rel(p))+'/' for p in paths)+')')
 core=fp(exact_paths(m.CORE_PLAN_FILES))
 stepdir=under(*m.STEP_DIRS)
 strategy=under(m.STRATEGY_DIR)
 lifecycle=fp(exact_paths(m.LIFECYCLE_FILES))
 helperpath=fp(exact_paths(m.RUNTIME_DISPATCH_HELPER_FILES))
 executor_scope=A(fp(exact_paths((*m.CORE_PLAN_FILES,*m.RUNTIME_DISPATCH_HELPER_FILES))),not_at(m.EXECUTOR_BYPASS_SCAN_FILE))
 primary_paths=sorted(set(rel(p) for p in (*m.CORE_PLAN_FILES,*m.BATCHING_SUPPORT_FILES,*m.LIFECYCLE_FILES,m.OPTIMIZER_ANALYSIS_FILE,m.CONDITIONAL_DIALECT_TEST_FILE)))
 directory_paths=[rel(p) for p in (*m.STEP_DIRS,m.STRATEGY_DIR)]
 primary_ast=O({'filePath':{'regex':'^(?:'+'|'.join(re.escape(p) for p in primary_paths)+')$'}},A(N({'filePath':{'regex':r'(?:^|/)__init__\.py$'}}),{'filePath':{'regex':'^(?:'+'|'.join(re.escape(p)+'/' for p in directory_paths)+r'|impl/common/l2070_server_plan/[^/]+$)'}}))
 fallback_ast=A(N({'filePath':{'regex':r'(?:^|/)__init__\.py$'}}),{'filePath':{'regex':'^(?:'+'|'.join(re.escape(rel(p))+'/' for p in m.SERVERPLAN_FALLBACK_SCAN_DIRS)+')'}})
 callback_ast=A(N({'filePath':{'regex':r'(?:^|/)__init__\.py$'}}),{'filePath':{'regex':r'^(?:impl|l[0-9]{4}_[^/]+)/'}})
 fallback_entries={'_check_conditional_serverplan_fallback','_check_projection_helper_execution_fallback','_check_materialized_row_semantics_fallback','_check_graphql_serverplan_boundary','_check_include_through_shared_lowering','_check_contract_first_relationship_runtime','_check_serverplan_cleanup_ratchets'}
 # Whole-source gates of the full analyzer, as module text.
 source_gates={
  '_check_compile_direct_io':modtext('compile','cursor|execute|fetch'),
  '_check_executor_python_execution':modtext(alt(m.EXECUTOR_RUNTIME_HELPERS)),
  '_check_conditional_serverplan_fallback':modtext(alt(m._SERVERPLAN_BUILD_CALLS),'execute|execute_async|query|query_async|load_with_includes'),
  '_check_projection_helper_execution_fallback':modtext(alt(m._PROJECTION_HELPER_NAME_TOKENS),'execute|execute_async|resolve|resolve_async'),
  '_check_materialized_row_semantics_fallback':modtext(alt(m._MATERIALIZED_ROW_SEMANTIC_CALL_NAMES|m._MATERIALIZED_ROW_COLLECTION_NAMES)),
 }
 clauses=[]
 # The entry point and the production domain select files (`ast.file`), before any node is
 # matched; the module-text gates of the full analyzer stay in the rule.
 primary_files='^(?:'+'|'.join(re.escape(p) for p in primary_paths)+')$|^(?:'+'|'.join(re.escape(p)+'/' for p in directory_paths)+r'|impl/common/l2070_server_plan/[^/]+$)'
 fallback_files='^(?:'+'|'.join(re.escape(rel(p))+'/' for p in m.SERVERPLAN_FALLBACK_SCAN_DIRS)+')'
 any_file={'path':{'regex':r'(?s).*'}}
 def file_gate(entry,files):
  es=(entry,) if isinstance(entry,str) else tuple(entry)
  if files is None:files=primary_files+('|'+fallback_files if es[0] in fallback_entries else '')
  domain={'path':{'regex':files,'notRegex':r'(?:^|/)__init__\.py$'}} if files else any_file
  return {'$option':'entryPoint','cases':{**{e:any_file for e in es},'_check_serverplan_quality_source':any_file,'all':choice('oracleAnalyzer',any_file,domain)},'default':{'path':{'regex':r'\b\B'}}}
 def text_gate(entry,main,production):
  es=(entry,) if isinstance(entry,str) else tuple(entry)
  main=both(main,source_gates.get(es[0],TRUE))
  if main==TRUE and production==TRUE:return None
  return {'$option':'entryPoint','cases':{'all':choice('oracleAnalyzer',main,both(main,production)),'_check_serverplan_quality_source':main},'default':TRUE}
 def query(entry,rule,main,files=None,production=TRUE):
  """`files`: the production domain as a path regex ('' for every file); `production`: its text gate."""
  gate=text_gate(entry,main,production)
  return {'language':'python','syntax':'valid','file':file_gate(entry,files),'rule':gated(rule,gate) if gate else rule}
 def forbid(id,entry,number,rule,main=TRUE,anchor='witness.span',**domain):
  clauses.append({'id':'spq-'+id,'forbid':{'ast':query(entry,rule,main,**domain)},
   'violation':{'code':'SPQ-'+str(number).zfill(3),'message':'Architecture contract violation','anchor':anchor}})
 def relation(id,entry,inputs,stages,rows,main=TRUE,admit=(),**domain):
  # A batch none of whose files pass the detector's gates skips the clause.
  relations={}
  for name,spec in {**inputs,'admission':(A(K('module'),*admit),{'path':'file.path'})}.items():
   relations[name]=E(TRUE,spec[1],**(spec[2] if len(spec)>2 else {}))
   relations[name]['evidence']['query']['ast'].update(query(entry,spec[0],main,**domain))
  clauses.append({'id':'spq-'+id,'relational':{'inputs':relations,'requireNonemptyInput':'admission','stages':stages,'query':' UNION ALL '.join(rows),'key':['issue','identity'],'code':'issue','partitionBy':'file'},
   'violation':{'code':'architecture','message':'Architecture contract violation','anchor':'witness.span'}})

 # SPQ-051/052/053: deleted callback surfaces, removed batch members, ORDERED_SEMANTIC inventory.
 callback_tokens=sorted(m.FORBIDDEN_CALLBACK_ARCHITECTURE_NAMES|{'CompiledStep','PipelineLifecycleEffect','pending_pipeline_effects'})
 callback=dict(files=primary_files+r'|^(?:impl|l[0-9]{4}_[^/]+)/',production=modtext(alt(callback_tokens)))
 inventory=dict(files='',production=modtext('MAINTENANCE|ORDERED_SEMANTIC'))
 entry='_check_deleted_callback_architecture_surfaces';names51=m.FORBIDDEN_CALLBACK_ARCHITECTURE_NAMES
 forbid('callback-names',entry,51,A(py_name(names51),NOT_TESTS),**callback)
 forbid('callback-members',entry,51,A(member(names51),NOT_TESTS),**callback)
 forbid('callback-kwargs',entry,51,A(K('call'),F('function',term(['CompiledStep'])),keyword(m.FORBIDDEN_COMPILED_STEP_CALLBACK_KWARGS),NOT_TESTS),**callback)
 forbid('callback-effect-fields',entry,51,A(K('assignment'),F('left',names(m.FORBIDDEN_PIPELINE_EFFECT_FIELD_NAMES)),class_field(A(K('class_definition'),base('PipelineLifecycleEffect'))),NOT_TESTS),**callback)
 forbid('callback-pending-lambdas',entry,51,A(K('lambda'),I(fn('pending_pipeline_effects')),NOT_TESTS),**callback)
 entry='_check_removed_maintenance_batch_architecture'
 for owner,member_name in sorted(m._REMOVED_BATCH_MEMBERS.items()):
  forbid('removed-batch-member-'+owner+'-'+member_name,entry,52,A(K('attribute'),RX(r'(?:^|\.)'+re.escape(owner)+r'\.'+re.escape(member_name)+'$')),**inventory)
  forbid('removed-batch-field-'+owner+'-'+member_name,entry,52,A(K('assignment'),F('left',names([member_name])),F('right',N(K('assignment'))),class_field(A(K('class_definition'),F('name',names([owner]))))),**inventory)
 # Occurrences of ORDERED_SEMANTIC are counted per innermost class and function scope.
 scope=modtext('ORDERED_SEMANTIC')
 limits=json.dumps([(*key,value) for key,value in sorted(m._ORDERED_SEMANTIC_SCOPE_LIMITS.items())],separators=(',',':'))
 innermost=lambda table:f"coalesce((SELECT s.name FROM {table} s WHERE s.__path=n.__path AND s.__start<=n.__start AND s.__end>=n.__end ORDER BY s.__end-s.__start LIMIT 1),'<module>')"
 relation('ordered-semantic-inventory','_check_ordered_semantic_batch_inventory',{
  'ordered':(A(member(['ORDERED_SEMANTIC']),{'filePath':{'notRegex':r'(?:^|/)tests(?:/|$)|(?:^|/)(?:data_history|build)(?:/|$)'}},F('attribute',P('ATTR'))),{'attr':'capture.ATTR'}),
  'scope_classes':(A(K('class_definition'),F('name',P('NAME')),scope),{'name':'capture.NAME'}),
  'scope_fns':(A(K('function_definition'),F('name',P('NAME')),scope),{'name':'capture.NAME'}),
 },[S('ordered_limits',"SELECT json_extract(value,'$[0]') AS path,json_extract(value,'$[1]') AS class_name,json_extract(value,'$[2]') AS function_name,json_extract(value,'$[3]') AS maximum FROM json_each("+q(limits)+")",('path','class_name','function_name')),
  S('ordered_occurrences',"SELECT *,row_number() OVER(PARTITION BY __path,class_name,function_name ORDER BY __start,__end DESC) AS occurrence FROM (SELECT n.__path,n.__start,n.__end,"+innermost('scope_classes')+" AS class_name,"+innermost('scope_fns')+" AS function_name FROM ordered n)")],
  ["SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,'SPQ-053' AS issue,n.__start AS identity FROM ordered_occurrences n WHERE n.occurrence>coalesce((SELECT maximum FROM ordered_limits lim WHERE lim.path=n.__path AND lim.class_name=n.class_name AND lim.function_name=n.function_name),0)"],
  admit=(RX('ORDERED_SEMANTIC'),),**inventory)

 serverplan_step=cls_base('ServerPlanStep')
 # SPQ-001: an `_async` method or module function duplicating its sync twin (the later definition
 # of that name); a pair of abstract methods is exempt.
 async_file=modtext('_async')
 cls_cap=A(K('class_definition'),P('CLS'))
 abstract=O(I(A(K('decorated_definition'),H(A(K('decorator'),H(A(O(K('identifier'),K('attribute')),RX(r'(?:^|\.)abstractmethod$')))))),'neighbor'),
  F('body',A(H(A(K('pass_statement'),nth_stmt(1))),N(H(A(STMT,nth_stmt(2)))))))
 relation('sync-async-duplication','_check_sync_async_duplication',{
  'pair_methods':(A(K('function_definition'),F('name',P('NAME')),O(I(body_of(cls_cap),'neighbor'),I(A(K('decorated_definition'),I(body_of(cls_cap),'neighbor')),'neighbor')),async_file),{'name':'capture.NAME','cls':'capture.CLS.startByte'}),
  'pair_functions':(A(MODULE_FN,F('name',P('NAME')),async_file),{'name':'capture.NAME'}),
  'long_functions':(A(K('function_definition'),F('body',H(A(STMT,nth_stmt(4)))),F('name',P('NAME')),async_file),{'name':'capture.NAME'}),
  'abstract_functions':(A(K('function_definition'),abstract,F('name',P('NAME')),async_file),{'name':'capture.NAME'}),
 },[S('last_methods',"SELECT * FROM pair_methods m WHERE m.__start=(SELECT max(o.__start) FROM pair_methods o WHERE o.__path=m.__path AND o.cls=m.cls AND o.name=m.name)",('__path','cls','name')),
  S('last_functions',"SELECT * FROM pair_functions m WHERE m.__start=(SELECT max(o.__start) FROM pair_functions o WHERE o.__path=m.__path AND o.name=m.name)",('__path','name'))],
  [row('SPQ-001',"pair_methods n JOIN last_methods sync ON sync.__path=n.__path AND sync.cls=n.cls AND sync.name=substr(n.name,1,length(n.name)-6)",
    "substr(n.name,-6)='_async' AND NOT EXISTS(SELECT 1 FROM abstract_functions f WHERE f.__path=n.__path AND f.__start IN (n.__start,sync.__start)) AND EXISTS(SELECT 1 FROM long_functions f WHERE f.__path=n.__path AND f.__start IN (n.__start,sync.__start))"),
   row('SPQ-001',"pair_functions n JOIN last_functions sync ON sync.__path=n.__path AND sync.name=substr(n.name,1,length(n.name)-6)",
    "substr(n.name,-6)='_async' AND EXISTS(SELECT 1 FROM long_functions f WHERE f.__path=n.__path AND f.__start IN (n.__start,sync.__start))")],admit=(RX('_async'),))
 forbid('intent-boilerplate','_check_intent_boilerplate',2,A(serverplan_step,LEVEL4,has_method(A(fn('compile'),F('body',H(A(K('raise_statement'),nth_stmt(1),
  H(O(names(['TypeError','NotImplementedError']),A(K('call'),F('function',names(['TypeError','NotImplementedError']))))))))))),stepdir)
 forbid('postop-hydration','_check_postop_hydration',3,A(fn('post_op'),nfp(r'(?:^|/)_compile_helpers\.py$'),H(A(K('call'),F('function',names(['convert_row_db_to_python']))),stop='end')),stepdir)
 forbid('duck-typed-context','_check_duck_typed_context',4,A(K('call'),F('function',names(['getattr','hasattr']))),both(core,not_at(m.CONDITIONAL_STEP_FILE)))
 forbid('versioned-forking','_check_versioned_forking',5,method('lower',serverplan_step,calls_in(['get_versioned_staging_ctx'])),stepdir)
 marker=H(member([m._RESOLVE_MARKER]),stop='end')
 forbid('result-resolution','_check_result_resolution_duplication',6,method('_execute_new_pipeline_async',A(K('class_definition'),has_method(A(fn('_execute_new_pipeline'),marker))),marker),both(core,not_at(m.CONDITIONAL_STEP_FILE)))
 lifecycle_store=A(K('assignment'),N(TYPED),F('left',member(m._LIFECYCLE_ATTRS)))
 forbid('lifecycle-methods','_check_lifecycle_entanglement',7,A(lifecycle_store,I(method(None,K('class_definition'),N(F('name',names(['__init__'])))))),lifecycle)
 forbid('lifecycle-functions','_check_lifecycle_entanglement',7,A(lifecycle_store,I(MODULE_FN)),lifecycle)
 forbid('unused-raw-data','_check_unused_raw_data',8,A(K('assignment'),N(TYPED),F('left',A(O(K('pattern_list'),K('tuple_pattern')),H(A(K('identifier'),RX('^_$'),{'nthChild':2})))),
  F('right',call(['_build_query_spec_and_raw']))),stepdir)
 forbid('compile-direct-io','_check_compile_direct_io',11,A(K('call'),F('function',member(m._COMPILE_IO_FORBIDDEN_ATTRS)),I(method('compile',serverplan_step))))
 # SPQ-012/013/018: executor runtime helpers and IR dispatch.
 executors=method(None,cls_named(['PlanExecutor','SqlBackendExecutor']))
 ir_name=lambda:A(K('identifier'),RX('(?i)ir$'),N(RX(exact(m.RUNTIME_EXECUTION_IR_TYPES))))
 ir_check=A(K('call'),F('function',names(['isinstance'])),arg(1,self_or(A(ir_name(),N(I(K('keyword_argument'),'neighbor','name'))))))
 execution='_check_executor_python_execution'
 forbid('executor-helper-functions',execution,12,A(K('function_definition'),F('name',names(m.EXECUTOR_RUNTIME_HELPERS)),executor_scope))
 forbid('executor-helper-methods',execution,12,method(m.EXECUTOR_RUNTIME_HELPERS,cls_named(['PlanExecutor','SqlBackendExecutor']),executor_scope))
 forbid('executor-protocols',execution,13,A(cls_named(['_ReplayStepLike','_ConditionalStepLike']),base('Protocol'),executor_scope))
 forbid('executor-helper-isinstance',execution,13,A(ir_check,helperpath,I(K('function_definition'))))
 forbid('executor-method-isinstance',execution,13,A(ir_check,executor_scope,I(executors)))
 forbid('executor-sequential-calls',execution,13,A(K('call'),F('function',member(['uses_runtime_sequential_execution','materialized_terminal_steps'])),executor_scope,I(executors)))
 forbid('executor-dispatch-calls',execution,12,A(K('call'),F('function',member(['_dispatch_runtime_ir','_dispatch_runtime_ir_async','_dispatch_runtime_ir_impl','_execute_compound_write','_execute_compound_write_async','_execute_sql_sequence','_execute_sql_sequence_async'])),executor_scope,I(executors)))
 forbid('executor-runtime-bypass',('_check_executor_execute_runtime_bypass',execution),13,A(K('call'),F('function',A(K('attribute'),RX(r'^object\.__getattribute__$'))),arg(1,str_lit(['execute_runtime'])),I(method(None,cls_named(['PlanExecutor'])))),
  O(at(m.EXECUTOR_BYPASS_SCAN_FILE),executor_scope))
 forbid('executor-child-steps',execution,13,A(member(['child_steps']),executor_scope,I(executors)))
 forbid('executor-runtime-results',execution,18,A(member(m.RUNTIME_RESULT_KEYWORDS),executor_scope,I(executors)))
 codegen=cls_base('PhysicalRegionCodegen')
 strategies='_check_runtime_sequential_strategies'
 forbid('strategy-sequential-optin',strategies,14,method('uses_runtime_sequential_execution',codegen,strategy,RETURNS_TRUE))
 forbid('strategy-unimplemented',strategies,14,method(['compile_batch','split_results'],codegen,strategy,calls_in(['NotImplementedError'])))
 forbid('strategy-sequential-probe',strategies,14,A(K('call'),F('function',names(['getattr'])),arg(1,str_lit(['requires_runtime_sequential_batch'])),strategy,I(method(None,codegen))))
 forbid('strategy-ir-calls',strategies,14,A(K('call'),F('function',O(ir_name(),A(K('attribute'),F('attribute',ir_name())))),strategy,I(method(None,codegen))))
 forbid('sequential-optins','_check_runtime_sequential_step_optins',15,method('requires_runtime_sequential_batch',serverplan_step,RETURNS_TRUE))
 forbid('dynamic-child-methods','_check_dynamic_child_steps',16,method(['child_steps','materialized_terminal_steps'],serverplan_step),modtext('child_steps'))
 forbid('dynamic-child-stores','_check_dynamic_child_steps',16,A(K('attribute'),F('attribute',A(K('identifier'),RX('(?:child_steps|materialized_terminal_steps)$'))),I(K('assignment'),'neighbor','left'),I(method(None,serverplan_step))),modtext('child_steps'))
 compiled_step=call(['CompiledStep','_CompiledStep'])
 for name in sorted(m.RUNTIME_RESULT_KEYWORDS):
  forbid('runtime-result-steps-'+name,'_check_runtime_result_compiled_steps',17,A(compiled_step,keyword([name],N(K('none'))),NOT_TESTS),modtext('runtime_result'))
 forbid('lazy-resolution','_check_imperative_lazy_resolution',20,A(K('call'),F('function',member(['resolve','resolve_async','result'])),F('arguments',N(H(A(NAMED,N(K('comment')),N(K('list_splat')),N(K('dictionary_splat'))))))))
 # SPQ-020 (pre-op): an imperative pre-op callback. Functions are imperative when they call a
 # runtime helper or, transitively, another imperative function of the same file.
 preop='_check_imperative_preop_execution'
 preop_file=modtext('pre_op','CompiledStep')
 forbid('preop-inline',preop,20,A(compiled_step,keyword(['pre_op','async_pre_op'],A(N(K('identifier')),N(K('none')),self_or(call(m.PRE_OP_RUNTIME_HELPERS)))),NOT_TESTS))
 relation('preop-functions',preop,{
  'preop_fns':(A(K('function_definition'),F('name',P('NAME')),preop_file),{'name':'capture.NAME'}),
  'preop_seeds':(A(K('function_definition'),calls_in(m.PRE_OP_RUNTIME_HELPERS),F('name',P('NAME')),preop_file),{'name':'capture.NAME'}),
  'preop_calls':(A(K('call'),F('function',O(A(K('identifier'),P('CALLEE')),A(K('attribute'),F('attribute',P('CALLEE'))))),preop_file),{'callee':'capture.CALLEE'}),
  'preop_steps':(A(compiled_step,keyword(['pre_op','async_pre_op'],A(K('identifier'),P('VALUE'))),NOT_TESTS,preop_file),{'value':'capture.VALUE'},{'each':'VALUE'}),
 },[S('imperative',"WITH RECURSIVE imperative(__path,name) AS (SELECT DISTINCT __path,name FROM preop_seeds UNION SELECT f.__path,f.name FROM preop_fns f JOIN preop_calls c ON c.__path=f.__path AND c.__start>f.__start AND c.__end<=f.__end JOIN imperative i ON i.__path=c.__path AND i.name=c.callee) SELECT * FROM imperative",('__path','name'))],
  [row('SPQ-020','preop_steps',"EXISTS(SELECT 1 FROM imperative i WHERE i.__path=n.__path AND i.name=n.value)")],admit=(RX('pre_op'),RX('CompiledStep')))
 conditional_step=method('compile',cls_named(['ConditionalStep']))
 sql_callee=F('function',O(A(K('identifier'),RX('^Sql')),A(K('attribute'),F('attribute',RX('^Sql')))))
 forbid('conditional-compile-ownership','_check_conditional_sql_compile_ownership',21,A(K('call'),at(m.CONDITIONAL_STEP_FILE),I(conditional_step),O(call(['compile']),sql_callee,A(call(['CompiledStep']),keyword(['grammar'])))))
 forbid('control-flow-child-compile','_check_control_flow_child_compile_calls',22,A(call(['compile']),not_at(m.CONDITIONAL_COMPILE_FILE),I(method('compile',A(cls_named(m.CONTROL_FLOW_STEP_TYPES|{'ConditionalStep'}),LEVEL4)))))
 forbid('dialect-test-seam','_check_conditional_dialect_test_compiler_seam',23,A(K('call'),at(m.CONDITIONAL_DIALECT_TEST_FILE),F('function',A(K('attribute'),F('attribute',names(['compile'])),F('object',names(['cs','conditional','conditional_step']))))))
 helpers=m.ORDER_BY_COMPILE_HELPER_NAMES;helper_file=at(m.COMPILE_HELPERS_FILE);order='_check_order_by_compile_helper_escape_hatches'
 forbid('order-by-names',order,24,A(py_name(helpers),helper_file))
 forbid('order-by-members',order,24,A(member(helpers),helper_file))
 forbid('order-by-classes',order,24,A(K('class_definition'),F('name',names(helpers)),helper_file))
 forbid('order-by-functions',order,24,A(K('function_definition'),F('name',names(helpers)),helper_file))
 quote=r"""(?:'''|\"\"\"|'|")"""
 forbid('order-by-strings',order,24,A(K('string'),RX(r'(?:^[rRuUfF]*'+quote+r'|[\s}])(?:'+alt(helpers)+r')(?:[\s{]|'+quote+'$)'),helper_file))
 lock=A(O(A(K('identifier'),*PY_NAME),K('attribute'),K('string'),K('concatenated_string')),RX('(?i)'+alt(m.LOCK_BATCHABILITY_NAMES)))
 forbid('can-batch-lock','_check_can_batch_lock_snapshot_policy',25,A(fn('can_batch'),F('body',H(O(A(K('if_statement'),F('condition',self_or(lock))),A(K('return_statement'),H(lock,stop='end')))))))
 forbid('branch-fetch-helper','_check_branch_fetch_batchability_helper',26,A(fn('_can_batch_fetch_branch_and_staged'),at(m.COMMIT_BRANCH_STEPS_FILE)))
 optimizer=at(m.OPTIMIZER_ANALYSIS_FILE);object_id='_check_plan_analysis_object_id_cache'
 forbid('object-id-calls',object_id,27,A(K('call'),F('function',names(['id'])),arg(0,names(['step'])),optimizer))
 forbid('object-id-names',object_id,27,A(py_name(['step_fact_step_ids']),optimizer))
 forbid('object-id-members',object_id,27,A(member(['step_fact_step_ids']),optimizer))
 analysis=cls_named(['PlanAnalysis'])
 field=A(K('attribute'),F('attribute',names(m.PLAN_ANALYSIS_MUTABLE_FIELDS)),O(F('object',names(['analysis'])),A(F('object',names(['self'])),I(analysis))))
 for kind in ('assignment','augmented_assignment'):
  forbid('plan-analysis-reassignment-'+kind,'_check_plan_analysis_field_reassignment',28,A(K(kind),F('left',self_or(field)),
   N(A(I(A(K('function_definition'),F('name',names(['__init__','build']))),stop=K('function_definition')),I(analysis)))),modtext('PlanAnalysis'))
 # SPQ-030/031: a branch choosing between building a ServerPlan and executing directly. A build call
 # names a build method or a ServerPlan/Intent segment on its dotted callee path; a direct call is an
 # execution method on a transaction-like receiver. Paths exist only for chains rooted at a name.
 chain_stop=N(O(K('attribute'),K('call')))
 rooted=N(H(A(N(O(K('attribute'),K('call'),K('identifier'))),O(I(K('attribute'),'neighbor','object'),I(K('call'),'neighbor','function'))),stop=chain_stop))
 build=A(K('call'),O(call(m._SERVERPLAN_BUILD_CALLS),H(A(K('identifier'),RX(r'^(?:ServerPlan|BaseServerPlan|[A-Za-z_]\w*Intent)$')),stop=chain_stop)),rooted)
 txn=['txn','tx','vrtxn','conn','query']
 segment=O(names(txn),member(txn))
 receiver=O(segment,A(K('call'),F('function',O(segment,A(K('call'),F('function',segment))))))
 includes=[v for v in m._DIRECT_EXECUTION_METHODS if v.lower().startswith('load_with_includes')]
 direct=A(K('call'),O(F('function',A(K('attribute'),F('attribute',names(m._DIRECT_EXECUTION_METHODS)),F('object',receiver))),call(includes)),rooted)
 def within(rule,kind):
  alternative=A(O(K('elif_clause'),K('else_clause')),H(rule,stop='end'))
  return H(alternative) if kind=='if_statement' else {'precedes':{**alternative,'stopBy':'end'}}
 builds_and_executes=A(K('function_definition'),H(build,stop='end'),H(direct,stop='end'))
 fallback='_check_conditional_serverplan_fallback'
 for kind in ('if_statement','elif_clause'):
  then_build=F('consequence',H(build,stop='end'));then_direct=F('consequence',H(direct,stop='end'))
  mixed=O(A(then_build,within(direct,kind)),A(within(build,kind),then_direct))
  forbid('serverplan-fallback-'+kind,fallback,30,A(K(kind),mixed,I(builds_and_executes)))
  none_test=A(K('comparison_operator'),RX(r'\bis(?: not)?\b'),H(K('none')),H(K('identifier')),N(H({'nthChild':3})))
  forbid('serverplan-none-fallback-'+kind,fallback,31,A(K(kind),N(mixed),O(then_direct,within(direct,kind)),F('condition',self_or(none_test)),I(builds_and_executes)))
 later=lambda rule:{'precedes':{**H(rule,stop='end'),'stopBy':'end'}}
 forbid('serverplan-later-fallback',fallback,30,A(K('if_statement'),O(A(F('consequence',H(build,stop='end')),later(direct)),A(F('consequence',H(direct,stop='end')),later(build))),
  I(body_of(builds_and_executes),'neighbor')))
 graphql=fp(r'(?:^|/)l9750_graphql/');graphql_exec=A(graphql,fp(r'(?:^|/)executor\.py$'));boundary='_check_graphql_serverplan_boundary'
 internals=m.GRAPHQL_FORBIDDEN_PLAN_INTERNALS
 forbid('graphql-internal-names',boundary,32,A(py_name(internals),graphql))
 forbid('graphql-internal-members',boundary,32,A(member(internals),graphql))
 forbid('graphql-internal-probes',boundary,32,A(K('call'),F('function',names(['getattr','hasattr'])),arg(1,str_lit(internals)),graphql))
 forbid('graphql-relationship-specs',boundary,33,A(K('class_definition'),F('name',A(K('identifier'),O(RX(exact(m.GRAPHQL_FORBIDDEN_RELATIONSHIP_SPEC_NAMES)),RX('(?i)RelationshipSpec$')))),graphql))
 forbid('graphql-relationship-helpers',boundary,33,A(call(m.GRAPHQL_FORBIDDEN_RELATIONSHIP_HELPERS),graphql))
 contracts=set(m.GRAPHQL_FORBIDDEN_EXECUTION_CONTRACTS)-set(internals)
 if contracts:forbid('graphql-execution-contracts',boundary,34,A(py_name(contracts),graphql_exec))
 # SPQ-035: an include loop that skips `through` relationships instead of sharing their lowering.
 through=str_lit(['through'])
 compared=A(K('comparison_operator'),RX('==|!='),H(A({'nthChild':'n+2'},self_or(through))))
 member_of=A(K('comparison_operator'),RX(r'\bin\b'),H(A({'nthChild':'n+2'},O(K('set'),K('list'),K('tuple')),H(through),N(H(A(N(through),LIT))))))
 intent_include=fp(r'(?:^|/)l2070_server_plan/.*intent_include\.py$')
 for kind in ('if_statement','elif_clause'):
  forbid('include-through-'+kind,'_check_include_through_shared_lowering',35,A(K(kind),intent_include,F('consequence',H(K('continue_statement'),stop='end')),F('condition',O(compared,member_of))))
 relationship=A(NOT_TESTS,fp(r'(?:^|/)(?:l9750_graphql|l9900_auto_api)/'))
 rest=fp(r'(?:^|/)l9900_auto_api/rest_router\.py$');contract='_check_contract_first_relationship_runtime'
 forbid('registry-relationships',contract,36,A(K('attribute'),F('attribute',names(['relationships'])),F('object',RX(r'(?:^|\.)registry(?:\.|$)')),relationship))
 forbid('relationship-lookups',contract,36,A(call(['get_relationship']),relationship))
 forbid('relationship-descriptor-calls',contract,36,A(call(['_relationship_descriptor']),rest,relationship))
 forbid('relationship-descriptor-sync',contract,36,A(fn('_relationship_descriptor'),NOT_ASYNC,rest,relationship))
 forbid('fk-planning-helpers',contract,37,A(call(m.GRAPHQL_FORBIDDEN_FK_PLANNING_HELPERS),relationship,graphql_exec))
 forbid('fk-planning-isinstance',contract,37,A(call(['isinstance']),arg(1,names(['BelongsTo','HasMany'])),relationship,graphql_exec))
 ratchets='_check_serverplan_cleanup_ratchets'
 legacy_names=['RelationshipInfo','relationship_type_from_contract'];legacy_files=nfp(r'(?:^|/)(?:schema_builder|__init__|graphql|relationship_compat)\.py$')
 forbid('relationship-info-names',ratchets,38,A(py_name(legacy_names),NOT_TESTS,legacy_files))
 forbid('relationship-info-imports',ratchets,38,A(K('import_from_statement'),F('module_name',RX(exact(['l9750_graphql.schema_builder','l9750_graphql.relationship_compat']))),F('name',import_named(legacy_names)),NOT_TESTS,legacy_files))
 forbid('prerequisite-members',ratchets,39,A(K('attribute'),O(F('attribute',names(['source_flush_context'])),A(F('attribute',names(['steps'])),F('object',names(['bundle'])))),NOT_TESTS))
 # One finding per keyword, as upstream reports each.
 for name in ('steps','source_flush_context'):forbid('prerequisite-bundles-'+name,ratchets,39,A(call(['PlanPrerequisiteBundle']),keyword([name]),NOT_TESTS))
 forbid('include-query-step',ratchets,40,A(call(['_build_include_query_step']),intent_include,I(A(fn('_lower_include_load_intent'),NOT_ASYNC))))
 forbid('embedded-object-sync',ratchets,41,A(fn('_format_embedded_object'),NOT_ASYNC,graphql_exec))
 forbid('embedded-object-calls',ratchets,41,A(call(['_format_embedded_object']),graphql_exec))
 # SPQ-042: a vector scope filter column given as a literal or a module string constant.
 vector='_check_vector_scope_filters_use_expressions';vector_file=at(m.VECTOR_QUERY_INTENT_FILE)
 filters=m.VECTOR_SCOPE_COLUMN_FILTERS
 query_args=A(K('argument_list'),I(call(['QueryFilter']),'neighbor','arguments'))
 column=O(A({'nthChild':{'position':1,'ofRule':POSARG}},I(query_args,'neighbor')),I(A(K('keyword_argument'),F('name',names(['column'])),I(query_args,'neighbor')),'neighbor','value'))
 forbid('vector-filter-literals',vector,42,A(str_lit(filters),vector_file,column))
 string_rule=A(O(A(K('string'),RX(r"(?i)^[ru]*['\"]"),N(I(K('concatenated_string'),'neighbor'))),A(K('concatenated_string'),N(H(K('interpolation'),stop='end')),N(H(A(K('string'),RX(r"(?i)^(?:fr|rf|f)['\"]")),stop='end')))),P('STRING'))
 decoded={'DECODED':{'replace':{'source':'$STRING','value':True,'replace':r'\b\B','by':''}}}
 relation('vector-filter-constants',vector,{
  'filter_names':(A(K('identifier'),P('NAME'),vector_file,column,modtext('QueryFilter')),{'name':'capture.NAME'}),
  'module_strings':(A(K('assignment'),N(TYPED),F('left',A(K('identifier'),P('NAME'))),F('right',string_rule),I(A(K('expression_statement'),I(K('module'),'neighbor')),'neighbor'),vector_file,modtext('QueryFilter')),
   {'name':'capture.NAME','value':'capture.DECODED.text'},{'transform':decoded}),
 },[S('module_string_values',"SELECT * FROM module_strings s WHERE s.__start=(SELECT max(o.__start) FROM module_strings o WHERE o.__path=s.__path AND o.name=s.name)",('__path','name'))],
  [row('SPQ-042','filter_names',"EXISTS(SELECT 1 FROM module_string_values s WHERE s.__path=n.__path AND s.name=n.name AND s.value IN "+literals(filters)+")")],admit=(vector_file,RX('QueryFilter')))
 # SPQ-043: a projection helper that executes and then repairs rows in a loop over its results.
 projection_dirs=under(m.ROOT/'l2060_query_builder',m.ROOT/'impl/common/l2070_server_plan',m.ROOT/'l2080_lazy_orm',m.ROOT/'l9900_auto_api')
 scoped_fn=lambda capture:I(A(K('function_definition'),P(capture)),stop=SCOPE_STOP)
 loop_name=lambda:A(K('identifier'),P('NAME'),*PY_NAME)
 refs=lambda:A(K('identifier'),P('NAME'),*PY_NAME)
 positional_dict=A(POSARG,I(A(K('call'),P('CALL'),F('function',names(['dict']))),'neighbor','arguments'))
 relation('projection-repairs','_check_projection_helper_execution_fallback',{
  'executing_fns':(A(K('function_definition'),projection_dirs,NOT_TESTS,H(A(K('call'),F('function',member(['execute','execute_async','resolve','resolve_async']))),stop=SCOPE_STOP),F('name',P('NAME'))),{'name':'capture.NAME'}),
  'statement_loops':(A(loop_name(),I(A(K('for_statement'),P('LOOP'),scoped_fn('FN')),field='left'),projection_dirs,NOT_TESTS),{'name':'capture.NAME','fn':'capture.FN.startByte','loop_start':'capture.LOOP.startByte','loop_end':'capture.LOOP.endByte'}),
  'comprehension_loops':(A(loop_name(),I(A(K('for_in_clause'),I(P('LOOP'),'neighbor'),scoped_fn('FN')),field='left'),projection_dirs,NOT_TESTS),{'name':'capture.NAME','fn':'capture.FN.startByte','loop_start':'capture.LOOP.startByte','loop_end':'capture.LOOP.endByte'}),
  'dict_refs':(A(refs(),O(positional_dict,I(positional_dict)),projection_dirs,NOT_TESTS),{'name':'capture.NAME','call_start':'capture.CALL.startByte','call_end':'capture.CALL.endByte'}),
  'dump_refs':(A(refs(),I(A(K('attribute'),I(A(K('call'),P('CALL'),F('function',member(['dict','model_dump']))),'neighbor','function')),field='object'),projection_dirs,NOT_TESTS),{'name':'capture.NAME','call_start':'capture.CALL.startByte','call_end':'capture.CALL.endByte'}),
 },[S('loop_names','SELECT __path,fn,loop_start,loop_end,name FROM statement_loops UNION SELECT __path,fn,loop_start,loop_end,name FROM comprehension_loops',('__path','name')),
  S('repaired',"SELECT DISTINCT l.__path,l.fn FROM loop_names l JOIN (SELECT __path,name,call_start,call_end FROM dict_refs UNION ALL SELECT __path,name,call_start,call_end FROM dump_refs) r ON r.__path=l.__path AND r.name=l.name AND r.call_start>=l.loop_start AND r.call_end<=l.loop_end",('__path','fn'))],
  [row('SPQ-043','executing_fns',"EXISTS(SELECT 1 FROM repaired r WHERE r.__path=n.__path AND r.fn=n.__start)")],admit=(projection_dirs,NOT_TESTS,RX('dict|model_dump'),RX('execute|resolve')))
 collections=m._MATERIALIZED_ROW_COLLECTION_NAMES
 semantic=O(A(K('call'),O(F('function',O(names(m._MATERIALIZED_ROW_SEMANTIC_CALL_NAMES),member(m._MATERIALIZED_ROW_SEMANTIC_CALL_NAMES))),F('function',member(['evaluate'])))),
  A(K('subscript'),F('value',names(collections)),H(K('slice'))))
 forbid('materialized-row-semantics','_check_materialized_row_semantics_fallback',44,A(K('function_definition'),NOT_TESTS,under(m.ROOT/'l9900_auto_api'),
  H(A(K('identifier'),RX(exact(collections)),*PY_NAME),stop=SCOPE_STOP),H(semantic,stop=SCOPE_STOP)))
 compound=nfp(r'(?:^|/)stmt_compound_write\.py$')
 forbid('sqlcompoundwrite-names','_check_forbidden_sqlcompoundwrite_usage',45,A(py_name(['SqlCompoundWrite']),NOT_TESTS,compound))
 forbid('sqlcompoundwrite-imports','_check_forbidden_sqlcompoundwrite_usage',45,A(K('import_from_statement'),F('module_name',RX(exact(['impl.common.l2100_sql_grammar.domain.infra.stmt_compound_write']))),F('name',import_named(['SqlCompoundWrite'])),NOT_TESTS,compound))
 batch='_check_sql_statement_batch_requires_category';statement_batch=call(['SqlStatementBatch'])
 forbid('statement-batch-category',batch,46,A(statement_batch,N(keyword(['category']))))
 forbid('statement-batch-rowcounts',batch,46,A(statement_batch,N(keyword(['statement_kinds_per_stmt'])),keyword(['collect_results_per_stmt'],A(O(K('list'),K('tuple')),H(K('true')))),
  keyword(['needs_returning_per_stmt'],A(O(K('list'),K('tuple')),N(H(A(NAMED,N(K('false')),N(K('comment')))))))))
 replay='_check_runtime_replay_batching_uses_structured_analysis'
 forbid('replay-fuse-terminal',replay,47,A(fn('_can_fuse_replay_terminal_steps'),NOT_ASYNC,fp(r'(?:^|/)sql_backend_executor\.py$')))
 forbid('replay-batch-transport',replay,47,A(fn('supports_sql_batch_transport'),calls_in(['sql_visible_dependencies'])))
 forbid('replay-outer-cte-strings',replay,47,A(K('string'),RX(r'^[rRuUfF]*'+quote+r'[\p{L}\p{N}_]+\(.*\)'+quote+'$'),N(H(K('interpolation'))),I(fn('split_for_outer_cte'))))
 dependent=fp(r'(?:^|/)dependent_read_cte\.py$')
 forbid('replay-relation-columns',replay,47,A(fn('_relation_output_columns'),dependent,O(H(member(['relation_output_columns']),stop='end'),H(A(call(['getattr']),F('arguments',H(str_lit(['relation_output_columns'])))),stop='end'))))
 forbid('replay-prior-boundary',replay,47,A(fn('requires_prior_step_boundary'),fp(r'(?:^|/)versioned_query_steps\.py$'),O(calls_in(['_query_spec_has_json_function_filter']),H(member(['_after_refs']),stop='end'))))
 forbid('replay-dependent-order',replay,47,A(K('module'),dependent,O(N(H(py_name(['_BATCH_ROW_ORDINAL']),stop='end')),N(calls_in(['with_order_by'])))),anchor='file.start')
 forbid('replay-child-exports',replay,47,A(K('module'),fp(r'(?:^|/)compound_sequence\.py$'),N(calls_in(['exports_child_bindings']))),anchor='file.start')
 batching_executor=fp(r'(?:^|/)l2070_batching/(?:.*/)?executor\.py$')
 none_store=F('consequence',H(A(K('expression_statement'),H(A(K('assignment'),N(TYPED),F('right',K('none')),F('left',K('subscript')))))))
 for kind in ('if_statement','elif_clause'):
  finish=A(K(kind),batching_executor,I(fn('_finish_batched_step')),none_store)
  forbid('replay-empty-result-'+kind,replay,47,A(finish,F('condition',self_or(py_name(['empty_result_allowed'])))))
  forbid('replay-omitted-projection-'+kind,replay,47,A(finish,F('condition',self_or(py_name(['omitted_projection_indices']))),N(F('condition',self_or(member(['closed_post_op']))))))
 tags=A(K('call'),F('function',member(['step_tags'])));kinds=A(K('set'),H(str_lit(['graph','versioned'])))
 forbid('replay-step-tags',replay,47,A(K('binary_operator'),batching_executor,RX('&'),O(F('left',tags),F('right',tags)),O(F('left',kinds),F('right',kinds))))
 forbid('batched-step-hint','_check_batched_step_construction_sets_batching_hint',48,A(K('function_definition'),fp(r'(?:^|/)batch_fusion\.py$'),calls_in(['PhysicalStatementGroup']),N(calls_in(['_set_batching_metrics_hint']))))
 from .quality_annotations import annotation_clauses
 annotation_clauses(m,forbid,relation)

 threshold=m.STRATEGY_SIZE_THRESHOLD
 # Python splitlines includes form feed, Unicode separators, and CRLF as one terminator.
 pattern=r'\A(?:[^\n\r\v\f\x1c-\x1e\x85  ]*(?:\r\n|[\n\r\v\f\x1c-\x1e\x85  ])){'+str(threshold)+r'}(?<OVERFLOW>[^\n\r\v\f\x1c-\x1e\x85  ]+|[\n\r\v\f\x1c-\x1e\x85  ])'
 query={'regex':{'source':'file.content','pattern':pattern,'paths':{'include':[rel(m.STRATEGY_DIR)+'/**/*.py'],'exclude':[]}}}
 empty={'regex':{'source':'file.content','pattern':r'(?<OVERFLOW>\b\B)'}}
 clauses.append({'id':'strategy-file-size','forbid':{'$option':'entryPoint','cases':{e:query for e in ['all','_check_serverplan_quality_source']},'default':empty},'violation':{'code':'SPQ-010','message':'Strategy file exceeds the line threshold','anchor':'capture.OVERFLOW'}})
 from .quality_taxonomy import taxonomy_clauses
 clauses.extend(taxonomy_clauses(m))
 return clauses
