#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate batch-three native policy documents from the pinned source constants."""
import argparse
import copy, importlib, json, re, sys
from pathlib import Path
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source',type=Path,required=True)
parser.add_argument('--repo',type=Path,required=True)
parser.add_argument('--output',type=Path,default=Path('packs/data-layer/candidates/rules'))
args=parser.parse_args()
SOURCE=args.source.resolve()
OUT=args.output.resolve()
OUT.mkdir(parents=True,exist_ok=True)
sys.path.insert(0,str(SOURCE))
PIN='2b1504adc4291daaf080b2e932962b134da1a69a'
from data_layer_batch_three import BATCH_THREE
from data_layer_probe_record import verify_sources
for check in BATCH_THREE:importlib.import_module('scripts.lint_'+check)
verify_sources(SOURCE,args.repo,PIN)
import subprocess
for artifact in ('.transaction-patterns-baseline.json','.swallowed-exceptions-baseline.json','scripts/allowlists/lint_module_level_caches_allowlist.json'):
 expected=subprocess.check_output(['git','-C',str(args.repo),'show',PIN+':'+artifact])
 if (SOURCE/artifact).read_bytes()!=expected:raise ValueError('Oracle artifact differs from pinned revision: '+artifact)
PRODUCTION=['impl/**/*.py','data_layer/**/*.py','_internal/**/*.py','l[0-9][0-9][0-9][0-9]_*/**/*.py']
EXCLUDE=['**/.git/**','**/.venv/**','**/venv/**','**/__pycache__/**','**/tmp/**','**/test/**','**/tests/**','**/test_*.py']
def A(*r):return {'all':list(r)}
def O(*r):return {'any':list(r)}
def N(r):return {'not':r}
def H(r,field=None,stop='neighbor'):return {'has':{**r,'stopBy':stop,**({'field':field} if field else {})}}
def I(r,stop='end',field=None):return {'inside':{**r,'stopBy':stop,**({'field':field} if field else {})}}
def F(f,r):return H(r,f)
def RX(r):return {'regex':r}
def K(k):return {'kind':k}
def names(values,kind='identifier'):return A(K(kind),RX('^(?:'+'|'.join(re.escape(x) for x in sorted(values))+')$' if values else r'\b\B'))
def term(values):return O(names(values),A(K('attribute'),F('attribute',names(values))))
def fn(name=None):return A(K('function_definition'),F('name',names([name]))) if name else K('function_definition')
FN=O(K('function_definition'),K('lambda'))
DECL=O(K('class_definition'),K('function_definition'),K('lambda'))
def C(id,rule,code,anchor='witness.span',each=None,file=None,when=None,message=None):
 ast={'language':'python','syntax':'valid','rule':rule}
 if file:ast['file']={'path':file}
 if when is not None:ast['when']={'option':when}
 v={'code':code,'anchor':anchor,'message':message or code+': '+id.replace('-',' ')}
 if each:v['each']=each
 return {'id':id,'forbid':{'ast':ast},'violation':v}
def R(id,pattern,code,paths=None,exclude=None,content=None,first=True):
 q={'source':'file.content','pattern':pattern,'lines':{'first':first}}
 if paths:q['paths']=paths
 if exclude:q['lines']['exclude']=exclude
 if content:q['content']={'include':[content]}
 return {'id':id,'forbid':{'regex':q},'violation':{'code':code,'anchor':'witness.span','message':code+': '+id}}
def W(name,clauses,utils=None,options=None,roots=None,exclude=None,mode='block'):
 if options is None:options={}
 options={'oracleAnalyzer':{'type':'boolean','default':False},**options}
 definition={'options':options}
 if utils:definition.update(language='python',utils=utils)
 doc={'format':'rerail.policy/v1','id':'data-layer-'+name.replace('_','-'),'revision':1,'mode':mode,
      'description':'Candidate port of nera-data-layer '+name+' at '+PIN+'.',
      'scope':{'include':roots or PRODUCTION,'exclude':EXCLUDE if exclude is None else exclude},
      'clauses':[{'id':'shared','define':definition},*clauses]}
 (OUT/(doc['id']+'.json')).write_text(json.dumps(doc,indent=2)+'\n')

def store_target():
 return O(I(O(K('assignment'),K('augmented_assignment'),K('for_statement'),K('for_in_clause')),'neighbor',field='left'),
  I(A(O(K('pattern_list'),K('tuple_pattern'),K('list_pattern'),K('tuple'),K('list'),K('list_splat_pattern'),K('parenthesized_expression')),{'matches':'store-target'}),'neighbor'),
  I(A(K('as_pattern_target'),I(A(K('as_pattern'),I(K('with_item'),'neighbor')),'neighbor',field='alias')),'neighbor'))

def nodecontains(text):
 return O(A(K('identifier'),RX(re.escape(text))),A(O(K('string'),K('concatenated_string')),{'value':{'regex':re.escape(text)}}),H({'matches':'contains-'+text}))
def before(c):return {'layout':{'after':{'to':'capture.'+c,'breaks':{'min':0}}}}
def after(c):return {'layout':{'before':{'to':'capture.'+c,'breaks':{'min':0}}}}
def direct_stmt(r):return A(r,I(K('block')),I(K('function_definition'),FN,field='body'))
# Cache scanner: exact stripped-line search, suffix-specific reviewed exceptions.
m=importlib.import_module('scripts.lint_connection_caches')
clauses=[]
allowed={}
for file,token,_ in m.ALLOWED_CACHES:allowed.setdefault(file,[]).append(re.escape(token))
for i,(pattern,label) in enumerate(m.ALL_PATTERNS):
 p=pattern.pattern
 if p.startswith('^'):p=r'^\s*'+p[1:]
 c=R('pattern-'+str(i),p,'CC001',paths={'include':['**/*.py'],'exclude':['**/*'+f for f in allowed]},exclude=[r'^\s*#'])
 c['violation']['message']=label
 clauses.append(c)
 for j,(file,tokens) in enumerate(allowed.items()):
  c=R('allowed-'+str(j)+'-pattern-'+str(i),p,'CC001',paths={'include':['**/*'+file],'exclude':[]},exclude=[r'^\s*#','|'.join(tokens)])
  # Preserve category identity across path partitions using the same clause id.
  c['id']='pattern-'+str(i)+'-file-'+str(j)
  c['violation']['message']=label
  clauses.append(c)
W('connection_caches',clauses,roots=['api/**/*.py','impl/**/*.py'])
# Error handling: only adjacent direct handler statements, and bare expression commit calls.
logger=A(K('expression_statement'),H(A(K('call'),F('function',A(K('attribute'),F('object',names(['logger'])),F('attribute',names(['warning','error'])))))))
raised=A(K('raise_statement'),N(H(O(K('identifier'),K('call'),K('attribute'),K('string'),K('integer'),K('none')))),
         {'follows':{**logger,'stopBy':K('expression_statement')}} ,I(K('block')),I(K('except_clause'),O(K('except_clause'),K('function_definition'),K('lambda'))))
# A sibling walk skips comments only, and stops at any other statement.
raised['all'][2]={'follows':{**logger,'stopBy':N(K('comment'))}}
commit=A(K('call'),{'pattern':'txn.commit()'},I(K('expression_statement')))
W('error_handling',[C('log-reraise',raised,'EH001',file={'regex':r'^(?:api|impl|services)'}),
 C('admin-commit',commit,'EH001',file={'regex':r'^api/l8000_admin/tools'})],roots=['api/**/*.py','impl/**/*.py','services/**/*.py'])
# Module-level cache declarations, including arbitrary control-flow blocks at module scope.
m=importlib.import_module('scripts.lint_module_level_caches')
module_scope=N(I(DECL))
name=A(K('identifier'),{'pattern':'$NAME'},N(names(['__all__'])),N(A(RX('^[^_].*_'),N(RX('[a-z]')))))
empty={'dict_cache':O(A(K('dictionary'),N(H(O(K('pair'),K('dictionary_splat'))))),{'pattern':'dict()'}),
       'set_cache':{'pattern':'set()'},
       'list_cache':O(A(K('list'),RX(r'(?s)^\[\s*(?:#[^\n]*\n\s*)*\]$')),{'pattern':'list()'})}
utils={}
clauses=[]
infrastructure={'notRegex':r'^(?:impl/common/)?l1000_cache/'}
for code,value in empty.items():
 clauses.append(C(code,A(K('assignment'),F('left',name),F('right',value),module_scope),code,each='NAME',file=infrastructure))
clauses.append(C('named-list',A(K('assignment'),F('left',A(name,RX('(?i)'+'|'.join(sorted(m.CACHE_NAME_TOKENS))))),
 F('right',A(K('list'),N(RX(r'(?s)^\[\s*(?:#[^\n]*\n\s*)*\]$')))),module_scope),'list_cache',each='NAME',file=infrastructure))
managed='^_*(?:'+'|'.join(sorted(m.MANAGED_CACHE_CONSTRUCTOR_NAMES))+')$'
constructor=O(term(m.CACHE_CONSTRUCTOR_NAMES),A(O(K('identifier'),K('attribute')),RX('Cache$')))
managed_ctor=O(A(K('identifier'),RX(managed)),A(K('attribute'),F('attribute',RX(managed))))
disabled=A(K('call'),F('arguments',H(A(K('keyword_argument'),F('name',names(['register_with_manager'])),F('value',K('false'))))))
clauses.append(C('constructor',A(K('assignment'),F('left',name),F('right',A(K('call'),F('function',constructor),
 N(A(F('function',managed_ctor),N(disabled))))),module_scope),'constructor_cache',each='NAME',file=infrastructure))
decorator_func=O(names(m.CACHE_DECORATORS),A(K('attribute'),F('object',names(['functools'])),F('attribute',names(m.CACHE_DECORATORS))))
utils['cache-decorator']=O(decorator_func,A(K('call'),F('function',{'matches':'cache-decorator'})))
callable_scope=A(N(I(FN)),N(I(A(K('class_definition'),I(K('class_definition'))))))
clauses.append(C('decorator',A(fn(),callable_scope,I(A(K('decorated_definition'),H(A(K('decorator'),{'pattern':'$DECORATOR'},H({'matches':'cache-decorator'})))),'neighbor')),'lru_cache',each='DECORATOR'))
W('module_level_caches',clauses,utils=utils,mode='block')
# Lazy-result methods: class authority, decorators, return annotations, and placeholder bodies.
m=importlib.import_module('scripts.lint_lazy_result_usage')
utils={}
utils['lazy-base']=O(term(['Protocol','ABC']),A(O(K('subscript'),K('generic_type')),H({'matches':'lazy-base'})))
clauses=[]
for method in sorted(m.SINGLE_VALUE_METHODS|m.SEQUENCE_METHODS):
 lazy_names=(method+'_lazy','execute_lazy') if method in {'first','one','one_or_none','select_one'} else (method+'_lazy',)
 lazyref=O(names(lazy_names),A(K('attribute'),F('attribute',names(lazy_names))))
 currentclass=A(K('class_definition'),F('name',A(RX('|'.join(sorted(m.TRANSACTION_CLASS_PATTERNS))),N(names(m.SKIP_CLASSES)),N(RX(r'^_*(?:Async|Lazy)')))),
  N(F('superclasses',H({'matches':'lazy-base'}))),
  N(F('body',H(O(fnname:=A(fn(),F('name',names(lazy_names))),A(K('decorated_definition'),H(fnname)))))))
 placeholder=F('body',A(K('block'),H(O(K('pass_statement'),A(K('expression_statement'),H(K('ellipsis'))))),
  N(H(A(N(K('comment')),{'nthChild':{'position':2,'ofRule':N(K('comment'))}})))))
 decorators=I(A(K('decorated_definition'),H(A(K('decorator'),H(O(term(['legacy_sync','legacy_async','abstractmethod']),A(K('call'),F('function',term(['legacy_sync','legacy_async','abstractmethod'])))))))),'neighbor')
 rule=A(fn(method),I(currentclass,K('class_definition')),N(F('return_type',RX('LazyResult|LazySeq|DeferredFuture'))),
  N(placeholder),N(decorators),N(H(lazyref,stop='end')))
 clauses.append(C(method,rule,'lazy_result_usage'))
W('lazy_result_usage',clauses,utils=utils)
# Plan-scope ownership combines nominal AST constraints and deliberately literal source checks.
m=importlib.import_module('scripts.lint_plan_scope_ownership')
utils={}
for text in ['Any','Versioned','CtePublicationContract','imported_contract']:
 utils['contains-'+text]=nodecontains(text)
utils['annotation-atom']=O(K('identifier'),A(O(K('type'),K('parenthesized_expression')),H({'matches':'annotation-atom'})))
runners= A(fn(),F('name',names(m.BASE_SCOPE_RUNNERS)))
param=O(K('identifier'),K('default_parameter'),K('typed_parameter'),K('typed_default_parameter'))
normal=A(param,N({'precedes':{**K('positional_separator'),'stopBy':'end'}}),N({'follows':{**O(K('keyword_separator'),K('list_splat_pattern'),K('dictionary_splat_pattern'),A(K('typed_parameter'),H(O(K('list_splat_pattern'),K('dictionary_splat_pattern'))))),'stopBy':'end'}}))
first=A(normal,{'nthChild':{'position':1,'ofRule':normal}})
second=A(normal,{'nthChild':{'position':2,'ofRule':normal}})
clauses=[C('runner-no-parameter',A(runners,N(F('parameters',H(normal)))),'PSO001')]
for async_ in [False,True]:
 selected=A(runners,F('name',RX('_async$')) if async_ else N(F('name',RX('_async$'))))
 exact=A({'matches':'annotation-atom'},RX('^'+('AsyncPlanScope' if async_ else 'SyncPlanScope')+'$'))
 clauses.append(C('runner-nominal-'+str(async_).lower(),A(selected,F('parameters',H(A(first,N(F('type',exact)))))),'PSO001'))
clauses += [C('runner-generic',A(runners,F('parameters',H(normal)),N(F('parameters',H(A(second,F('type',O(K('generic_type'),H(K('subscript'),stop='end'),RX(r'\[')))))))),'PSO008'),
 C('runner-broad',A(runners,F('parameters',H(A(first,F('type',RX('Any|object|Versioned|\\||Union')))))),'PSO001')]
for method,code in [(('getattr','hasattr','callable'),'PSO002'),(m.BASE_TERMINAL_FALLBACKS,'PSO003')]:
 call=A(K('call'),F('function',term(method) if code=='PSO002' else A(K('attribute'),F('attribute',names(method)))),I(A(runners,{'pattern':'$RUNNER'})))
 clauses.append(C('runner-'+code.lower(),call,code,each='RUNNER'))
for cls in ['SyncReadPlanWorkflow','SyncWritePlanWorkflow','AsyncReadPlanWorkflow','AsyncWritePlanWorkflow']:
 expected=cls.removesuffix('PlanWorkflow')+'Transaction'
 annotation=A(F('type',RX(r'Callable\s*\[\s*\[')),F('type',RX(re.escape(expected))),N(F('type',{'matches':'contains-Any'})))
 clauses.append(C('workflow-'+cls,A(K('assignment'),F('left',names(['build'])),F('type',K('type')),N(annotation),
  I(A(K('class_definition'),F('name',names([cls]))),DECL)),'PSO008'))
clauses.append(C('versioned-terminal',A(fn(),F('name',names(m.BASE_SCOPE_METHODS)),I(A(K('class_definition'),F('name',names(['SyncVersionedPlanScope','AsyncVersionedPlanScope']))),DECL)),'PSO005'))
clauses.append(C('helper-base-call',A(K('call'),F('function',term(m.BASE_SCOPE_RUNNERS))),'PSO004',file={'regex':r'^(?:conform/versioned_plan_scope_helpers\.py|data_layer_testing/versioning/query_helpers\.py)$'}))
owned={'regex':'^(?:'+'|'.join(re.escape(x) for x in sorted(m.PLAN_SCOPE_OWNERSHIP_FILES))+')$'}
clauses += [C('nested-terminal',A(K('call'),F('function',term(m.NESTED_VERSIONED_TERMINALS)),I(A(fn(),F('name',RX('nested_versioned'))),K('function_definition'))),'PSO006',file=owned),
 C('lifecycle-reflection',A(K('call'),F('function',term(['getattr','hasattr'])),O(*(O(A(K('identifier'),RX(re.escape(name))),H(A(K('identifier'),RX(re.escape(name))),stop='end'),H(A(O(K('string'),K('concatenated_string')),{'value':{'regex':re.escape(name)}}),stop='end')) for name in sorted(m.LIFECYCLE_NAMES)))),'PSO007',file=owned),
 C('base-versioned-name',A(K('call'),{'matches':'contains-Versioned'}),'PSO009',file={'regex':r'connection_plan_workflows\.py$'}),
 C('invent-contract',A(K('call'),F('function',{'matches':'contains-imported_contract'}),{'matches':'contains-CtePublicationContract'}),'PSO013')]
utils['store-target']=store_target()
store=A(K('attribute'),F('attribute',names(['_callback_versioned_txn'])),{'matches':'store-target'})
clauses.append(C('callback-owner',A(store,N(I(A(fn(),F('name',names(m.CALLBACK_OWNER_WRITERS))),K('function_definition')))),'PSO010'))
# Text checks run only when the fact framework accepts the source.
text_patterns=[('terminal-state','if not txn.is_committed and not txn.is_rolled_back and not txn.did_publish_branch','PSO011'),
 ('contract-fallback','imported_contract = imported_contract_fn','PSO013')]
for id,text,code in text_patterns:
 rule=A(K('module'),RX(re.escape(text)))
 if id=='contract-fallback':rule=A(rule,RX('else None'))
 # Regex groups supply the original literal's first location; each is capped to one by the module witness.
 rule=A(rule,RX('(?<LOCATION>'+re.escape(text)+')'))
 clauses.append(C(id,rule,code,anchor='capture.LOCATION'))
patterns=("execute_read_plan_workflow_async(\n            conn,\n            SyncReadPlanWorkflow","execute_write_plan_workflow_async(\n            conn,\n            SyncWritePlanWorkflow","run_read_plan_workflow_async(\n            self._async_conn(),\n            SyncReadPlanWorkflow","run_write_plan_workflow_async(\n            self._async_conn(),\n            SyncWritePlanWorkflow","execute_read_plan_workflow(\n        conn,\n        AsyncReadPlanWorkflow","execute_write_plan_workflow(\n        conn,\n        AsyncWritePlanWorkflow")
for i,p in enumerate(patterns):clauses.append(C('mode-pair-'+str(i),A(K('module'),RX('(?<LOCATION>'+re.escape(p)+')')),'PSO012',anchor='capture.LOCATION'))
W('plan_scope_ownership',clauses,utils=utils,roots=PRODUCTION+['conform/**/*.py','data_layer_testing/**/*.py'],exclude=EXCLUDE[:-1])
# Expression identifiers exclude attribute labels and keyword names, as Python ast.Name does.
def pyname():return A(K('identifier'),N(I(K('attribute'),'neighbor',field='attribute')),N(I(K('keyword_argument'),'neighbor',field='name')))
def same(a,b):return {'compare':{'left':a,'right':b,'op':'eq','text':'node'}}
# Eager effects inside the nearest explicit plan scope; audit reads retain sticky source-order names.
m=importlib.import_module('scripts.lint_plan_scope_side_effects')
utils={}
plan_call=A(K('call'),F('function',term(m.PLAN_OPENERS)))
write_call=A(K('call'),F('function',term(m.WRITE_PLAN_OPENERS)))
def item_of(call):return A(K('with_item'),F('value',O(call,A(K('as_pattern'),H(call)))))
utils['plan']=A(K('with_statement'),H(A(K('with_clause'),H(item_of(plan_call)))))
utils['write-plan']=A(K('with_statement'),H(A(K('with_clause'),H(item_of(write_call)))))
plan=I({'matches':'plan'},{'matches':'plan'})
write_plan=I(A({'matches':'write-plan'},{'pattern':'$PLAN'}),{'matches':'plan'})
receiver_names='(?i)^(?:'+'|'.join(sorted(m.SIDE_EFFECT_RECEIVERS))+')$'
utils['side-receiver']=O(A(K('identifier'),RX(receiver_names)),A(K('attribute'),O(F('attribute',RX(receiver_names)),F('object',{'matches':'side-receiver'}))),
 A(K('call'),F('function',{'matches':'side-receiver'})),A(K('subscript'),F('value',{'matches':'side-receiver'})))
utils['typed-read']=A(K('call'),F('function',term(m.TYPED_EAGER_QUERY_TERMINALS)))
utils['direct-read']=O({'matches':'typed-read'},A(K('parenthesized_expression'),H({'matches':'direct-read'})))
utils['assigned-read-name']=O(A(K('identifier'),{'pattern':'$READ_NAME'}),A(O(K('pattern_list'),K('tuple_pattern'),K('list_pattern'),K('tuple'),K('list')),H({'matches':'assigned-read-name'})))
prior_assignment=A(K('assignment'),F('left',{'matches':'assigned-read-name'}),F('right',{'matches':'typed-read'}),before('IF'),
 I(A({'matches':'write-plan'},{'pattern':'$ASSIGN_PLAN'},same('PLAN','ASSIGN_PLAN')),{'matches':'plan'}))
read_name=A(pyname(),{'pattern':'$READ_NAME'},I(A(K('if_statement'),{'pattern':'$IF'},write_plan,
 I(A(K('module'),H(prior_assignment,stop='end')))),K('if_statement')))
# The helper is evaluated at a condition's names, after the enclosing if and plan have been captured.
utils['read-test']=O(A(pyname(),{'pattern':'$READ_NAME'},I(A(K('module'),H(prior_assignment,stop='end')))),H({'matches':'read-test'}))
clauses=[C('eager-value',A(K('attribute'),F('attribute',names(m.EAGER_VALUE_ATTRS)),plan),'PSE-010'),
 C('eager-call',A(K('call'),F('function',term(m.EAGER_CALLS)),plan),'PSE-020'),
 C('raw-database',A(K('call'),F('function',term(m.RAW_DB_CALLS)),plan),'PSE-030'),
 C('legacy-callback',A(K('call'),F('function',A(K('attribute'),F('attribute',names(m.LEGACY_CALLBACK_METHODS)))),plan),'PSE-040'),
 C('python-side-effect',A(K('call'),F('function',O(names(['open']),A(K('attribute'),F('attribute',names(m.SIDE_EFFECT_CALLS)),F('object',{'matches':'side-receiver'})))),plan),'PSE-050'),
 C('branch-audit',A(K('if_statement'),{'pattern':'$IF'},write_plan,F('condition',O({'matches':'direct-read'},{'matches':'read-test'}))),'PSE-AUDIT-010',when={'oracleAnalyzer':True})]
W('plan_scope_side_effects',clauses,utils=utils)
# Transaction callback wrappers: current function calls and immediate named callback ownership.
m=importlib.import_module('scripts.lint_transaction_callback_wrappers')
utils={}
utils['contains-Callable']=nodecontains('Callable')
utils['contains-Transaction']=nodecontains('Transaction')
utils['callback-annotation']=A({'matches':'contains-Callable'},{'matches':'contains-Transaction'})
paramannot=A(O(K('typed_parameter'),K('typed_default_parameter')),F('type',{'matches':'callback-annotation'}))
entrycall=A(K('call'),F('function',A(K('attribute'),F('attribute',names(m.TRANSACTION_ENTRYPOINTS)))))
reentrant=O(A(K('call'),F('function',A(K('attribute'),F('attribute',names(m.REENTRANT_VERSIONED_ENTRYPOINTS))))),
 A(K('call'),F('function',names(['getattr'])),F('arguments',H(A(O(K('string'),K('concatenated_string')),{'value':{'regex':'^(?:'+'|'.join(sorted(m.REENTRANT_VERSIONED_ENTRYPOINTS))+')$'}},N({'nthChild':{'position':1,'ofRule':N(K('comment'))}}))))))
# Match the callback expression selected by keyword precedence, then by second positional argument.
def callback_arg(selected):
 kw=A(K('keyword_argument'),F('name',names(['fn'])),F('value',selected))
 pos=A(selected,N(K('keyword_argument')),{'nthChild':{'position':2,'ofRule':A(N(K('comment')),N(K('keyword_argument')),N(K('dictionary_splat')))}})
 return F('arguments',O(H(kw),A(N(H(A(K('keyword_argument'),F('name',names(['fn']))))),H(pos))))
boundary=A(F('name',names(m.TRANSACTION_ENTRYPOINTS)),I(A(K('class_definition'),F('name',names(['SyncDatabaseConnection','SyncVersionedDatabaseConnection']))),K('class_definition')))
base=A(fn(),{'pattern':'$OWNER'},N(boundary))
allowed={'notRegex':'(?:^|/)(?:'+ '|'.join(re.escape(p) for p in sorted(m.ALLOWED_RELATIVE_PATHS))+')$'}
# Lambda bodies stay in the current function; nested function definitions reset the collector context.
helper=A(base,F('parameters',H(paramannot)),F('body',H(entrycall,stop=K('function_definition'))))
named_callback=A(entrycall,callback_arg(A(K('identifier'),{'pattern':'$CALLBACK_NAME'})))
nested=A(fn(),F('name',A(K('identifier'),{'pattern':'$CALLBACK_NAME'})),
 I(A(K('block'),I(A(fn(),{'pattern':'$CHECK_OWNER'},same('OWNER','CHECK_OWNER')),'neighbor')),'neighbor'),
 F('body',H(reentrant,stop=K('function_definition'))))
named_reentry=A(base,F('body',H(named_callback,stop=K('function_definition'))),F('body',H(nested)))
lambda_reentry=A(base,F('body',H(A(entrycall,callback_arg(A(K('lambda'),F('body',O(reentrant,H(reentrant,stop=K('function_definition'))))))),stop=K('function_definition'))))
clauses=[C('callback-helper',helper,'transaction_callback_wrapper',file=allowed),
 C('inline-reentrant',O(named_reentry,lambda_reentry),'transaction_callback_wrapper',file=allowed)]
W('transaction_callback_wrappers',clauses,utils=utils)
# Swallowed exception facts: handler-local raises and exact legacy/current suppress multiplicity.
m=importlib.import_module('scripts.lint_swallowed_exceptions')
utils={}
single_protocol=A(K('tuple'),H({'matches':'protocol'}),N(H(A(N(K('comment')),{'nthChild':{'position':2,'ofRule':N(K('comment'))}}))))
utils['protocol']=O(names(['KeyError','StopIteration','StopAsyncIteration']),{'pattern':'queue.Empty'},A(K('parenthesized_expression'),H({'matches':'protocol'})),single_protocol)
handler=K('except_clause')
stop=O(DECL,handler)
# Once a nested scope is entered under an outer handler, the original collector ignores raises/references.
nested_under_handler=I(A(DECL,I(handler)))
body_raise=H(A(K('block'),H(K('raise_statement'),stop=stop)))
raised=A(N(nested_under_handler),body_raise)
ref=A(N(nested_under_handler),O(F('alias',A(K('identifier'),{'pattern':'$ERROR'})),F('value',A(K('as_pattern'),F('alias',H(A(K('identifier'),{'pattern':'$ERROR'})))))),
 O(H(A(K('block'),H(A(pyname(),{'pattern':'$ERROR'}),stop=stop))),F('value',O(A(pyname(),{'pattern':'$ERROR'}),H(A(pyname(),{'pattern':'$ERROR'}),stop=O(stop,K('as_pattern_target')))))))
base=A(handler,N(F('value',O({'matches':'protocol'},A(K('as_pattern'),H({'matches':'protocol'}))))),N(raised))
clauses=[C('except-warning',A(base,ref),'WARNING'),C('except-critical',A(base,N(ref)),'CRITICAL')]
legacy_func=O(names(['suppress']),{'pattern':'contextlib.suppress'})
args1protocol=F('arguments',A(H({'matches':'protocol'}),N(H(A(N(K('comment')),{'nthChild':{'position':2,'ofRule':N(K('comment'))}})))))
legacy=A(K('call'),{'pattern':'$SUPPRESS'},F('function',legacy_func),N(args1protocol))
with_legacy=A(K('with_statement'),N(RX(r'^async\b')),H(A(K('with_clause'),H(item_of(legacy)))))
clauses.append(C('suppress-legacy',with_legacy,'CRITICAL',each='SUPPRESS'))
fromcontextlib=A(K('import_from_statement'),F('module_name',names(['contextlib'],'dotted_name')))
importsuppress=A(fromcontextlib,{'pattern':'$IMPORT'},H(O(A(K('dotted_name'),RX('^suppress$')),A(K('aliased_import'),F('name',A(K('dotted_name'),RX('^suppress$')))))),before('WITH'),
 N({'precedes':{**A(fromcontextlib,before('WITH')),'stopBy':'end'}}))
# Search any later import before WITH, including imports in another syntactic scope.
importsuppress['all'][-1]=N(I(A(K('module'),H(A(fromcontextlib,{'pattern':'$LATER'},after('IMPORT'),before('WITH')),stop='end'))))
bare=A(names(['suppress']),I(A(K('module'),H(importsuppress,stop='end'))))
currentfunc=O({'pattern':'contextlib.suppress'},bare)
keyerroronly=F('arguments',A(H(names(['KeyError'])),N(H(A(N(K('comment')),{'nthChild':{'position':2,'ofRule':N(K('comment'))}})))))
current=A(K('call'),{'pattern':'$SUPPRESS'},F('function',currentfunc),N(keyerroronly))
clauses.append(C('suppress-critical',A(K('with_statement'),{'pattern':'$WITH'},H(A(K('with_clause'),H(item_of(current))))),'CRITICAL',each='SUPPRESS'))
W('swallowed_exceptions',clauses,utils=utils,roots=['api/**/*.py','impl/**/*.py','services/**/*.py','l[0-9][0-9][0-9][0-9]_*/**/*.py'],mode='steer')
# Connection-parameter classification. Category lists are configuration, not Python callbacks.
m=importlib.import_module('scripts.lint_transaction_patterns')
utils={}
leaf_has=O(A(K('identifier'),RX('Connection')),A(O(K('string'),K('concatenated_string')),{'value':{'regex':'Connection'}}))
leaf_conn=O(A(K('identifier'),RX('Connection|^conn$'),N(names(['ConnectionId']))),
 A(O(K('string'),K('concatenated_string')),{'value':{'regex':'Connection|^conn$'}},N({'value':{'eq':'ConnectionId'}})))
def left(u):return O(F('left',{'matches':u}),A(K('union_type'),H(A(K('type'),{'nthChild':1},{'matches':u}))))
def right(u):return O(F('right',{'matches':u}),A(K('union_type'),H(A(K('type'),{'nthChild':2},{'matches':u}))))
for name,leaf in [('annotation-has-connection',leaf_has),('annotation-connection',leaf_conn)]:
 recur={'matches':name}
 binary=O(K('binary_operator'),K('union_type'))
 branch=O(A(left('annotation-has-connection'),left(name)),A(N(left('annotation-has-connection')),right('annotation-has-connection'),right(name)))
 utils[name]=O(leaf,A(K('attribute'),F('attribute',recur)),A(K('subscript'),F('value',recur)),
  A(K('generic_type'),H(A(K('identifier'),recur))),A(O(K('type'),K('parenthesized_expression')),H(recur)),A(binary,branch))
param_name=O(A(K('typed_parameter'),H(A(K('identifier'),N(names(['self'])),{'pattern':'$CONN'}))),
 A(K('typed_default_parameter'),F('name',A(K('identifier'),N(names(['self'])),{'pattern':'$CONN'}))))
conn_param=A(normal,F('type',{'matches':'annotation-connection'}),param_name)
conn_param_count=A(normal,F('type',{'matches':'annotation-connection'}),O(A(K('typed_parameter'),H(A(K('identifier'),N(names(['self']))))),
 A(K('typed_default_parameter'),F('name',A(K('identifier'),N(names(['self'])))))))
first_conn=A(conn_param,{'nthChild':{'position':1,'ofRule':conn_param_count}})
base=A(fn(),F('parameters',H(first_conn)))
internal=F('body',H(A(K('call'),F('function',A(K('attribute'),F('object',A(K('identifier'),{'pattern':'$CONN'})),F('attribute',names(['read','write','schema_transaction']))))),stop=K('function_definition')))
utils['connection-function']=base
base={'matches':'connection-function'}
utils['internal-transaction']=internal
internal={'matches':'internal-transaction'}
init=A(fn('__init__'),I(A(K('class_definition'),F('name',RX('|'.join(sorted(m.ALLOWED_INIT_CLASS_PATTERNS))))),K('class_definition')))
presets=m._get_categorized_functions(m._load_baseline(SOURCE))
categories=list(dict.fromkeys([*presets,'NEEDS_REVIEW','TS_BRIDGE','INTERNAL_TXN_VIOLATION']))
options={}
for cat in categories:
 options['category-'+cat]={'type':'array','items':'string','default':sorted(presets.get(cat,set()))}
 utils['category-'+cat]=F('name',A(K('identifier'),{'regex':{'$option':'category-'+cat,'escape':'regex','join':'|','template':'^(?:{})$'}}))
baseline_any=O(*({'matches':'category-'+cat} for cat in categories))
clauses=[]
allowed_internal={'LEGITIMATE','BOOTSTRAP','BACKGROUND','BACKEND_OPS','VALIDATION','ADMIN_TOOLS'}
serial=0
def emit_category(category, rule, file=None):
 global serial
 serial+=1
 gate=None if category in m._FAILING_CATEGORIES else {'oracleAnalyzer':True}
 if category not in allowed_internal:
  clauses.append(C('category-'+str(serial),A(base,rule,N(internal)),category,file=file,when=gate))
  clauses.append(C('internal-'+str(serial),A(base,rule,internal),'INTERNAL_TXN_VIOLATION',file=file))
 else:clauses.append(C('category-'+str(serial),A(base,rule),category,file=file,when=gate))
emit_category('LEGITIMATE',init)
for cat in categories:emit_category(cat,A(N(init),{'matches':'category-'+cat}))
utils['unclassified-function']=A(N(init),N(baseline_any))
unknown={'matches':'unclassified-function'}
# Each ordered OR condition is partitioned into a name branch and a disjoint path branch.
previous_names=[];previous_paths=[]
def decision(category,name=None,path=None, domain=None):
 global previous_names,previous_paths
 if name:
  rule=A(unknown,F('name',RX(name)),*(N(F('name',RX(p))) for p in previous_names))
  excluded=previous_paths + ([domain] if domain else [])
  emit_category(category,rule,{'notRegex':'|'.join('(?:'+p+')' for p in excluded)} if excluded else None)
 if path:
  rule=A(unknown,*(N(F('name',RX(p))) for p in previous_names+([name] if name else [])))
  file={'regex':path}
  if previous_paths:file['notRegex']='|'.join('(?:'+p+')' for p in previous_paths)
  emit_category(category,rule,file)
 if name:previous_names.append(name)
 if path:previous_paths.append(path)
# Deleted TS bridge directories retain their legacy purpose-based inference when explicitly scanned.
ts=r'(?:l9900_ts_models|l0500_ts_models)'
ts_cases=[('BOOTSTRAP',r'initialize|ensure|bootstrap|create_log_table'),('BACKGROUND',r'ripple|reaper|background|process_'),
 ('CAPABILITY',r'table_exists|get_table|list_tables|introspect'),('BULK_OPS',r'export_|import_|validate_import'),
 ('BOOTSTRAP',r'rbac|permission'),('LEGITIMATE',r'schema_transaction|begin_')]
ts_prev=[]
for cat,rx in ts_cases:
 emit_category(cat,A(unknown,F('name',RX(rx)),*(N(F('name',RX(p))) for p in ts_prev)),{'regex':ts})
 ts_prev.append(rx)
emit_category('BACKEND_OPS',A(unknown,*(N(F('name',RX(p))) for p in ts_prev)),{'regex':ts})
previous_paths.append(ts)
decision('ADMIN_TOOLS',r'_handler',r'(?:^|/)tools/|^l8000_admin/')
decision('VALIDATION',r'(?i)validat',r'validation')
decision('BOOTSTRAP',r'ensure|initialize|(?i:bootstrap)')
decision('BACKGROUND',r'stale|reaper|recover|ripple|process_|enqueue_ripple')
decision('CAPABILITY',r'_detect_|get_capabilities|has_native|get_backend_type|run_after_rollback|attach_renewable_capability_runtime',r'^l6510_credentials/')
decision('SCHEMA_OPS',r'register_queue|register_table|sync_column|register_trigger')
decision('BOOTSTRAP',path=r'^l(?:3100_schema_introspector|3200_metadata|3250_migrations)/')
decision('SUBSCRIPTION',r'subscribe')
decision('BULK_OPS',r'export_database|import_database',r'bulk')
decision('BACKEND_SPECIFIC',path=r'^impl/.*(?:sqlite|postgres|mariadb|mongodb|neo4j|doltdb)')
decision('BACKEND_OPS',path=r'^(?:api|impl|data_layer|_internal|l\d{4}_[^/]+)/')
emit_category('NEEDS_REVIEW',A(unknown,*(N(F('name',RX(p))) for p in previous_names)),{'notRegex':'|'.join('(?:'+p+')' for p in previous_paths)})
W('transaction_patterns',clauses,utils=utils,options=options)
# Transaction closure mutation checks. Both compatibility visitors remain independently replayable.
m=importlib.import_module('scripts.lint_transaction_closures')
utils={}
transaction=A(K('call'),{'pattern':'$INVOKE'},F('function',A(K('attribute'),F('attribute',names(m.TRANSACTION_METHODS)))))
def param_named(capture):
 token=A(K('identifier'),{'pattern':'$'+capture})
 ordinary=A(normal,O(token,F('name',token),H(token)))
 splat=O(A(O(K('list_splat_pattern'),K('dictionary_splat_pattern')),H(token)),
  A(K('typed_parameter'),H(A(O(K('list_splat_pattern'),K('dictionary_splat_pattern')),H(token)))))
 return O(ordinary,splat)
def simple_param(capture):
 token=A(K('identifier'),{'pattern':'$'+capture})
 return A(normal,O(token,F('name',token),A(K('typed_parameter'),H(token))))
def assignment_name(capture):
 token=A(K('identifier'),{'pattern':'$'+capture})
 return O(A(K('assignment'),F('left',token)),A(K('named_expression'),F('name',token)))
def plain_assignment_name(capture):return A(K('assignment'),F('left',A(K('identifier'),{'pattern':'$'+capture})))
utils['module-assignment']=O(plain_assignment_name('OBJECT'),A(K('assignment'),F('right',{'matches':'module-assignment'})))
module_binding=O(A(K('expression_statement'),H({'matches':'module-assignment'})),
 A(O(K('function_definition'),K('class_definition')),F('name',A(K('identifier'),{'pattern':'$OBJECT'}))),
 A(K('decorated_definition'),H(A(O(K('function_definition'),K('class_definition')),F('name',A(K('identifier'),{'pattern':'$OBJECT'}))))),
 A(O(K('import_statement'),K('import_from_statement')),H(O(A(K('aliased_import'),F('alias',A(K('identifier'),{'pattern':'$OBJECT'}))),
  A(K('dotted_name'),N(I(K('aliased_import'),'neighbor')),N(I(K('import_from_statement'),'neighbor',field='module_name')),
   H(A(K('identifier'),{'pattern':'$OBJECT'},{'nthChild':1})))))))
inline_outer=O(I(A(fn(),O(F('parameters',H(param_named('OBJECT'))),H(assignment_name('OBJECT'),stop='end')))),I(A(K('module'),H(module_binding))))
inline_lambda=A(K('lambda'),{'pattern':'$CLOSURE'},I(A(transaction,callback_arg(A(K('lambda'),{'pattern':'$LAMBDA'},same('CLOSURE','LAMBDA')))),'end'))
inline_local=O(F('parameters',H(param_named('OBJECT'))),F('body',O(O(A(K('assignment'),F('left',A(K('identifier'),{'pattern':'$OBJECT'},before('MUTATION')))),A(K('named_expression'),F('name',A(K('identifier'),{'pattern':'$OBJECT'},before('MUTATION'))))),H(O(A(K('assignment'),F('left',A(K('identifier'),{'pattern':'$OBJECT'},before('MUTATION')))),A(K('named_expression'),F('name',A(K('identifier'),{'pattern':'$OBJECT'},before('MUTATION'))))),stop=K('lambda')))))
inline_context=I(A(inline_lambda,N(inline_local),inline_outer),K('lambda'),field='body')
# Named callbacks are direct inner functions, with the last matching definition chosen before each call.
call_named=A(transaction,callback_arg(A(K('identifier'),{'pattern':'$CALLBACK_NAME'})),after('CLOSURE'),
 I(A(fn(),{'pattern':'$USE_OWNER'},same('OUTER','USE_OWNER')),K('function_definition')))
named_root=A(fn(),{'pattern':'$CLOSURE'},F('name',A(K('identifier'),{'pattern':'$CALLBACK_NAME'})),
 I(A(K('block'),I(A(fn(),{'pattern':'$OUTER'},F('body',H(call_named,stop=K('function_definition')))),'neighbor')),'neighbor'))
later_definition=A(fn(),F('name',A(K('identifier'),{'pattern':'$CALLBACK_NAME'})),after('CLOSURE'))
named_root=A(named_root,N(I(A(fn(),{'pattern':'$CHECK_OUTER'},same('OUTER','CHECK_OUTER'),F('body',H(later_definition))),'end')))
# Local assignments and nested function parameters accumulate in source traversal order.
local_binding=O(O(A(K('assignment'),F('left',A(K('identifier'),{'pattern':'$OBJECT'},before('MUTATION')))),A(K('named_expression'),F('name',A(K('identifier'),{'pattern':'$OBJECT'},before('MUTATION'))))),
 A(fn(),F('parameters',H(A(param_named('OBJECT'),before('MUTATION'))))))
named_local=O(F('parameters',H(param_named('OBJECT'))),F('body',H(local_binding,stop=K('lambda'))))
outer_binding=O(F('parameters',H(simple_param('OBJECT'))),F('body',H(O(A(K('expression_statement'),H({'matches':'module-assignment'})),plain_assignment_name('OBJECT')))))
named_outer=I(A(fn(),{'pattern':'$VARIABLE_OWNER'},same('OUTER','VARIABLE_OWNER'),outer_binding))
named_context=I(A(named_root,N(named_local),named_outer),K('lambda'),field='body')
named_plain_context=I(named_root,K('lambda'),field='body')
mutating=A(K('call'),{'pattern':'$MUTATION'},F('function',A(K('attribute'),F('object',A(K('identifier'),{'pattern':'$OBJECT'})),F('attribute',names(m.MUTATING_METHODS)))))
# ast.Store applies to the subscript itself, rather than a read in another assignment target.
utils['store-target']=O(A(O(K('assignment'),K('augmented_assignment'),K('for_statement'),K('for_in_clause')),F('left',{'pattern':'$STORE'})),
 A(O(K('pattern_list'),K('tuple_pattern'),K('list_pattern')),I({'matches':'store-target'},'neighbor')))
store=A(K('subscript'),{'pattern':'$MUTATION'},F('value',A(K('identifier'),{'pattern':'$OBJECT'})),
 O(I(A(O(K('assignment'),K('augmented_assignment'),K('for_statement'),K('for_in_clause')),F('left',{'pattern':'$STORE'},)), 'neighbor'),
  I(A(O(K('pattern_list'),K('tuple_pattern'),K('list_pattern')),I({'matches':'store-target'},'neighbor')),'neighbor')))
# Direct parent-field identity excludes read subscripts on the RHS.
store=A(K('subscript'),{'pattern':'$MUTATION'},F('value',A(K('identifier'),{'pattern':'$OBJECT'})),
 I(A(O(K('assignment'),K('augmented_assignment'),K('for_statement'),K('for_in_clause')),F('left',A({'pattern':'$TARGET'},same('MUTATION','TARGET')))),'neighbor'))
utils['inline-context']=inline_context
utils['named-context']=named_context
utils['named-plain-context']=named_plain_context
inline_context={'matches':'inline-context'}
named_context={'matches':'named-context'}
named_plain_context={'matches':'named-plain-context'}
utils['store-target']=store_target()
store=A(K('subscript'),{'pattern':'$MUTATION'},F('value',A(K('identifier'),{'pattern':'$OBJECT'})),{'matches':'store-target'})
clauses=[C('inline-mutation',A(mutating,inline_context),'TXN003',each='INVOKE'),
 C('inline-subscript',A(store,inline_context),'TXN004',each='INVOKE'),
 C('named-nonlocal',A(K('identifier'),I(K('nonlocal_statement'),'neighbor'),named_plain_context),'TXN001',each='INVOKE'),
 C('named-global',A(K('identifier'),I(K('global_statement'),'neighbor'),named_plain_context),'TXN002',each='INVOKE'),
 C('named-mutation',A(mutating,named_context),'TXN003',each='INVOKE'),
 C('named-subscript',A(store,named_context),'TXN004',each='INVOKE')]
W('transaction_closures',clauses,utils=utils)
# Production allowlists remain data: partition each affected check by exact path and declaration name.
p=OUT/'data-layer-module-level-caches.json';doc=json.loads(p.read_text());expanded=[]
allowlist=json.loads((SOURCE/'scripts/allowlists/lint_module_level_caches_allowlist.json').read_text())['entries']
for clause in doc['clauses']:
 if 'forbid' not in clause:expanded.append(clause);continue
 root_rule=clause['forbid']['ast']['rule']
 if root_rule.get('all',[{}])[0].get('kind')=='assignment':
  for term_ in root_rule['all']:
   spec=term_.get('has',{})
   if spec.get('field')=='right':
    value={key:val for key,val in spec.items() if key not in ('field','stopBy')}
    utility='assignment-value-'+clause['id']
    doc['clauses'][0]['define']['utils'][utility]=O(value,A(K('assignment'),F('right',{'matches':utility})))
    term_['has']={'field':'right','stopBy':'neighbor','matches':utility}
 raw=copy.deepcopy(clause);raw['id']='raw-'+raw['id'];raw['forbid']['ast']['when']={'option':{'oracleAnalyzer':True}};expanded.append(raw)
 base=copy.deepcopy(clause);base['forbid']['ast']['when']={'option':{'oracleAnalyzer':False}}
 bypath={}
 for entry in allowlist:
  if entry['pattern_type']==clause['violation']['code']:bypath.setdefault(entry['path'],[]).append(entry['name'])
 if not bypath:expanded.append(base);continue
 default=copy.deepcopy(base)
 paths=default['forbid']['ast'].setdefault('file',{}).setdefault('path',{})
 paths['notRegex']='|'.join([paths.get('notRegex',r'\b\B'),'^(?:'+'|'.join(re.escape(path) for path in bypath)+')$'])
 expanded.append(default)
 for index,(path,names_) in enumerate(sorted(bypath.items())):
  branch=copy.deepcopy(base);branch['id']+='-file-'+str(index)
  branch['forbid']['ast'].setdefault('file',{}).setdefault('path',{})['regex']='^'+re.escape(path)+'$'
  exemptions=[]
  for name_ in names_:
   if clause['violation']['code']=='lru_cache':
    if '.' in name_:
     cls,method=name_.rsplit('.',1);exemptions.append(A(F('name',names([method])),I(A(K('class_definition'),F('name',names([cls]))),K('class_definition'))))
    else:exemptions.append(A(F('name',names([name_])),N(I(K('class_definition')))))
   else:exemptions.append(F('left',names([name_])))
  branch['forbid']['ast']['rule']=A(branch['forbid']['ast']['rule'],N(O(*exemptions)))
  expanded.append(branch)
doc['clauses']=expanded;p.write_text(json.dumps(doc,indent=2)+'\n')
# FactEngine's source triggers and explicit source/AST APIs have different entry-point contracts.
triggers={
 'connection_caches':importlib.import_module('scripts.lint_connection_caches').CONNECTION_CACHES_EXTRACTOR.source_triggers,
 'error_handling':importlib.import_module('scripts.lint_error_handling').ERROR_HANDLING_EXTRACTOR.source_triggers,
 'module_level_caches':importlib.import_module('scripts.lint_module_level_caches').MODULE_LEVEL_CACHES_EXTRACTOR.source_triggers,
 'plan_scope_ownership':importlib.import_module('scripts.lint_plan_scope_ownership').PLAN_SCOPE_OWNERSHIP_FACT.source_triggers,
 'plan_scope_side_effects':importlib.import_module('scripts.lint_plan_scope_side_effects').PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.source_triggers,
 'transaction_callback_wrappers':importlib.import_module('scripts.lint_transaction_callback_wrappers').TRANSACTION_CALLBACK_WRAPPER_EXTRACTOR.source_triggers,
 'transaction_patterns':importlib.import_module('scripts.lint_transaction_patterns')._TRANSACTION_PATTERN_SOURCE_TRIGGERS,
 'transaction_closures':importlib.import_module('scripts.lint_transaction_closures').TRANSACTION_CALL_TRIGGERS,
 'swallowed_exceptions':importlib.import_module('scripts.lint_swallowed_exceptions').SWALLOWED_EXCEPTIONS_EXTRACTOR.source_triggers,
}
for name,words in triggers.items():
 p=OUT/('data-layer-'+name.replace('_','-')+'.json');doc=json.loads(p.read_text())
 doc['clauses'][0]['define']['options']['sourceGate']={'type':'boolean','default':True}
 pattern='|'.join(re.escape(word) for word in sorted(words))
 option={'$option':'sourceGate','cases':{'true':pattern,'false':'^'}}
 gate=O(A(K('module'),{'regex':option}),I(A(K('module'),{'regex':option})))
 for clause in doc['clauses']:
  ast_=clause.get('forbid',{}).get('ast')
  if ast_ is not None:ast_['rule']=A(ast_['rule'],gate)
  q=clause.get('forbid',{}).get('regex')
  if q is not None:q['content']={'$option':'sourceGate','cases':{'true':{'include':[pattern]},'false':{}}}
 p.write_text(json.dumps(doc,indent=2)+'\n')
p=OUT/'data-layer-swallowed-exceptions.json';doc=json.loads(p.read_text());doc['scope']['include']=[root+'/**/*.py' for root in importlib.import_module('scripts.lint_swallowed_exceptions')._SCAN_DIRS];p.write_text(json.dumps(doc,indent=2)+'\n')
p=OUT/'data-layer-plan-scope-ownership.json';doc=json.loads(p.read_text());doc['scope']['exclude']=['**/'+part+'/**' for part in sorted(importlib.import_module('scripts.lint_plan_scope_ownership').SKIP_DIR_NAMES)];p.write_text(json.dumps(doc,indent=2)+'\n')
p=OUT/'data-layer-lazy-result-usage.json';doc=json.loads(p.read_text());skip='|'.join(re.escape(part) for part in importlib.import_module('scripts.lint_lazy_result_usage').SKIP_PATTERNS)
for clause in doc['clauses']:
 if 'forbid' in clause:clause['forbid']['ast']['file']={'path':{'notRegex':skip}}
p.write_text(json.dumps(doc,indent=2)+'\n')
p=OUT/'data-layer-connection-caches.json';doc=json.loads(p.read_text())
for clause in doc['clauses']:
 if 'forbid' in clause:clause['forbid']['regex']['paths']['exclude'].extend('**/'+part+'/**' for part in sorted(importlib.import_module('scripts.lint_connection_caches').SKIP_PARTS))
p.write_text(json.dumps(doc,indent=2)+'\n')
