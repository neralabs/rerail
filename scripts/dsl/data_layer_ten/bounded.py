# SPDX-License-Identifier: Apache-2.0
"""Preserve upstream textual architecture checks, including absent artifacts."""
from .common import *
import re,ast

def candidate(root):
 c=constants(root/'scripts/lint_bounded_composed_graph_paths.py');paths={n.targets[0].id:n.value.args[0].value for n in ast.parse((root/'scripts/lint_bounded_composed_graph_paths.py').read_text()).body if isinstance(n,ast.Assign) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Name) and n.value.func.id=='Path' and len(n.value.args)==1 and isinstance(n.value.args[0],ast.Constant)};rows=[];required=[]
 def src(key):return 'replace(replace(source_text('+q(paths[key])+'),char(13)||char(10),char(10)),char(13),char(10))'
 def rx(text,pattern,group=0):return f'rx_extract({text},{q(pattern)},{group})'
 def matches(text,pattern):return f'coalesce(rx_matches({text},{q(pattern)}),0)'
 def line(prefix):return f"1+length({prefix})-length(replace({prefix},char(10),''))"
 def prefix(text,pattern):return rx(text,r'(?ms)\A(.*?)'+pattern,1)
 def block(key,name,method=False):
  header=(r'^    (?:async )?def '+re.escape(name)+r'(?:\[[^\n]*\])?\s*\(') if method else (r'^class '+re.escape(name)+r'(?:\[[^\n]*\])?(?:\([^\n]*\))?:')
  tail=rx(src(key),r'(?ms)('+header+r'.*)',1)
  headertext=rx(tail,r'(?s)\A('+header.removeprefix('^')+')',1)
  rest=f'substr({tail},length({headertext})+1)'
  stop=r'^    (?:@|(?:async )?def )' if method else r'^(?:class |def |async def )'
  return f'({headertext}||{rx(rest,r"(?ms)\A(.*?)(?:"+stop+r"|\z)",1)})',line(prefix(src(key),header))
 def emit(code,key,message,condition='1',ln='1'):
  ordinal=len(rows);p=q(paths[key]);text=src(key)
  rows.append(f"SELECT {p} path,0 start_byte,coalesce(length(CAST(source_text({p}) AS BLOB)),0) end_byte,{ln} line,{q(code)} issue,{q(message)} message,{ordinal} identity,0 phase,{ordinal} ordering,'' symbol,'' sort_name WHERE {condition}")
 def require(key):
  required.append(key);emit('BCGP001',key,'required architecture file is missing',src(key)+' IS NULL')
 def decorated(key,owner,method,deco,code,message):
  b,_=block(key,owner);pattern=rf'(?m)^    @{deco}(?:\([^\n]*\))?\n(?:    @[^\n]+\n)*    (?:async )?def {method}(?:\[[^\n]*\])?\s*\('
  emit(code,key,message,'NOT '+matches(b,pattern))
 for key in ['VERSIONED_LAZY_MIXIN','VERSIONED_TRANSACTIONS']:
  require(key)
  for name in sorted(c['BANNED_TRANSACTION_ENDPOINTS']):
   pattern=rf'(?m)^\s*(?:async )?def {name}\s*\('
   emit('BCGP002',key,f'permissionless or redundant transaction endpoint {name}() must remain deleted',matches(src(key),pattern),line(prefix(src(key),pattern)))
 require('VERSIONED_LAZY_MIXIN');decorated('VERSIONED_LAZY_MIXIN','VersionedReadTransactionLazyMixin','graph_lazy','pure_lazy','BCGP003','graph_lazy() must be the @pure_lazy graph entrypoint')
 require('VERSIONED_TRANSACTIONS')
 for name,deco in [('graph','legacy_sync'),('graph_async','legacy_async')]:decorated('VERSIONED_TRANSACTIONS','LazyVersionedReadTransaction',name,deco,'BCGP003',f'{name}() must delegate through @{deco}')
 require('GRAPH_PATTERN')
 for name in sorted(c['REQUIRED_PATH_SYMBOLS']):emit('BCGP004','GRAPH_PATTERN',f'required path semantic type {name} is missing',src('GRAPH_PATTERN')+' IS NOT NULL AND NOT '+matches(src('GRAPH_PATTERN'),rf'(?m)^(?:class|def|async def)\s+{name}(?:[^A-Za-z0-9_]|\z)'))
 b,ln=block('GRAPH_PATTERN','PathPattern')
 emit('BCGP005','GRAPH_PATTERN','PathPattern must own a complete PathExpression',b+' IS NOT NULL AND NOT '+matches(b,r'(?m)^    expression\s*:\s*([^\n=]+)(?:\s*=\s*([^\n]+))?'),ln)
 emit('BCGP005','GRAPH_PATTERN','edge-shaped PathPattern field must be removed',matches(b,r'(?m)^    edge\s*:\s*([^\n=]+)(?:\s*=\s*([^\n]+))?'),ln)
 require('QUERY_BUILDER')
 for name,deco in [('execute_paths_lazy','pure_lazy'),('execute_paths','legacy_sync'),('execute_paths_async','legacy_async')]:decorated('QUERY_BUILDER','GraphQueryBuilder',name,deco,'BCGP006',f'{name}() must exist with @{deco}')
 require('GRAPH_LAZY_SQL');decorated('GRAPH_LAZY_SQL','SqlPlanGraphQueryBuilder','execute_paths_lazy','pure_lazy','BCGP007','SQL plan builder must implement @pure_lazy execute_paths_lazy()')
 require('INTENT_GRAPH')
 for name in ['ExecuteGraphPathsStep','GraphPathEvidenceRelationRow']:
  b,_=block('INTENT_GRAPH',name);emit('BCGP008','INTENT_GRAPH',f'required ServerPlan path contract {name} is missing',src('INTENT_GRAPH')+' IS NOT NULL AND '+b+' IS NULL')
 b,ln=block('INTENT_GRAPH','ExecuteGraphPathsStep')
 for method,msg in [('cte_publication_contract','path step must publish an exact CTE relation contract'),('compile','path step must compile the backend relation')]:emit('BCGP008','INTENT_GRAPH',msg,b+' IS NOT NULL AND NOT '+matches(b,rf'(?m)^    (?:async )?def {method}(?:\[[^\n]*\])?\s*\('),ln)
 require('GRAPH_RELATION_PLAN')
 for name in ['BoundedComposedPathRelationPlan','GraphPathRelationSchemas']:
  b,_=block('GRAPH_RELATION_PLAN',name);emit('BCGP008','GRAPH_RELATION_PLAN',f'required semantic relation contract {name} is missing',src('GRAPH_RELATION_PLAN')+' IS NOT NULL AND '+b+' IS NULL')
 require('SQL_GRAPH_QUERY_BUILDER');b,_=block('SQL_GRAPH_QUERY_BUILDER','_build_execute_paths_relation_plan',True);emit('BCGP008','SQL_GRAPH_QUERY_BUILDER','bounded paths must build a semantic relation plan before SQL lowering',b+' IS NULL')
 b,_=block('SQL_GRAPH_QUERY_BUILDER','_build_execute_paths_grammar',True);emit('BCGP008','SQL_GRAPH_QUERY_BUILDER','bounded path SQL lowering must consume the semantic relation plan',f"coalesce(instr({b},'_build_execute_paths_relation_plan()'),0)=0")
 for key,names in [('GRAPH_LAZY_SQL',['SqlGraphExecutionCtx']),('INTENT_GRAPH',['_ResolveGraphExecutionCtxConfig','_GraphExecuteConfig']),('GRAPH_TRAVERSE_STEP',['_GraphTraversalApiConfig','_GraphTraversalTerminalConfig'])]:
  require(key)
  for name in names:
   b,ln=block(key,name);field=rx(b,r'(?m)^    graph_type\s*:\s*([^\n=]+)(?:\s*=\s*([^\n]+))?',1);default=rx(b,r'(?m)^    graph_type\s*:\s*([^\n=]+)(?:\s*=\s*([^\n]+))?',2)
   emit('BCGP009',key,f'required graph contract {name} is missing',src(key)+' IS NOT NULL AND '+b+' IS NULL')
   emit('BCGP009',key,f'{name} must carry graph_type',b+' IS NOT NULL AND '+field+' IS NULL',ln)
   emit('BCGP009',key,f'{name}.graph_type must be required and non-optional',f"instr({field},'None')>0 OR "+matches(default,r'^[\s\x1c-\x1f]*None[\s\x1c-\x1f]*$'),ln)
 require('SQL_GRAPH_QUERY_BUILDER')
 for token,description in [('_try_build_sparse_union_recursive_bindings_grammar','legacy sparse-union recursion'),('_try_build_sparse_recursive_bindings_grammar','legacy one-edge recursion'),('_SqlSparseUnionRecursiveBindingGrammar','legacy sparse binding grammar'),('_SqlSparseUnionRecursiveBindingFinalSelect','legacy sparse binding final selection')]:
  pattern='(SELECT left_word||'+q(token)+'||right_word FROM python_word_boundaries)'
  prefix_pattern="(SELECT '(?s)\\A(.*?'||left_word||')'||"+q(token)+'||right_word FROM python_word_boundaries)'
  condition='rx_matches('+src('SQL_GRAPH_QUERY_BUILDER')+','+pattern+')'
  prefix_value='rx_extract('+src('SQL_GRAPH_QUERY_BUILDER')+','+prefix_pattern+',1)'
  emit('BCGP014','SQL_GRAPH_QUERY_BUILDER',description+' must not bypass the relation-algebra path',condition,line(prefix_value))
 require('SPARQL_EXECUTOR');emit('BCGP010','SPARQL_EXECUTOR','SPARQL must acquire builders through graph_lazy(...).query_builder(...)',f"instr({src('SPARQL_EXECUTOR')},'graph_query_builder')>0")
 require('SPARQL_AST')
 for name in sorted(c['REQUIRED_SPARQL_PATH_SYMBOLS']):emit('BCGP010','SPARQL_AST',f'required SPARQL path type {name} is missing',src('SPARQL_AST')+' IS NOT NULL AND NOT '+matches(src('SPARQL_AST'),rf'(?m)^(?:class|def|async def)\s+{name}(?:[^A-Za-z0-9_]|\z)'))
 require('SPARQL_COMPILER');emit('BCGP010','SPARQL_COMPILER','SPARQL compiler must target the shared path expression contract',src('SPARQL_COMPILER')+" IS NOT NULL AND instr("+src('SPARQL_COMPILER')+",'PathExpression')=0")
 for key in ['DESIGN_DOC','IMPLEMENTATION_PLAN','RULE_DOC']:emit('BCGP011',key,'required architecture document is missing',src(key)+' IS NULL')
 for key in ['GRAPH_DOC','VERSIONED_DOC','SPARQL_DOC']:
  require(key);emit('BCGP011',key,'canonical documentation still advertises the removed direct builder endpoint',f"instr({src(key)},'graph_query_builder(')>0")
 require('LINT_ALL');patterns=[r'''name\s*=\s*['"]bounded_composed_graph_paths['"]''',r'''script\s*=\s*['"]lint_bounded_composed_graph_paths\.py['"]''',r'''enforcement\s*=\s*['"]hard-fail['"]''',r'in_ci\s*=\s*True'];emit('BCGP012','LINT_ALL','completion linter is not a hard-fail CI member of lint_all','NOT ('+' AND '.join(matches(src('LINT_ALL'),p) for p in patterns)+')')
 emit('BCGP013','CONFORMANCE_TEST','maintained-backend composed recursion contract is missing',src('CONFORMANCE_TEST')+' IS NULL')
 clauses=[]
 for index,query in enumerate(rows):
  clauses.append({'id':'text-contract-'+str(index),'relational':{'inputs':{'files':{'evidence':{'as':'f','query':{'regex':{'source':'file.content','pattern':r'(?s)\A(?P<SOURCE>.*)\z'}},'project':{'path':'file.path'}}}},'query':query,'locations':list(paths.values()),'key':['identity'],'line':'line','code':'issue','message':'message'},'violation':{'code':'architecture','message':'Architecture contract violation','anchor':'witness.span'}})
  if 'python_word_boundaries' in query:clauses[-1]['relational']['stages']=[S('python_word_boundaries','SELECT '+q(PY_LEFT)+' left_word,'+q(PY_RIGHT)+' right_word')]
 return make_document('bounded_composed_graph_paths',clauses,list(paths.values()),exclude=[])
