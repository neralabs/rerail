# SPDX-License-Identifier: Apache-2.0
"""Legacy execution inventories, receiver tokens, and eager-call reachability.

Local checks are syntax clauses. Only the checks that join independently matched sets (a file's
boundary status, a wrapper's other calls, import aliases, eager-call reachability) are relational,
and they read narrow candidate rows (functions, classes, calls, imports), never a copy of the tree.
"""
from .common import *

# Receiver tokens are the identifiers on a receiver's own chain: attribute objects and names, call
# callees, subscript values, parenthesized expressions and a cast's value. Other arguments and
# subscript indexes are not.
CHAIN_STOP=O(N(O(K('attribute'),K('call'),K('subscript'),K('parenthesized_expression'))),I(K('subscript'),'neighbor','subscript'))
def tokens(pattern):
 ident=A(K('identifier'),RX(pattern))
 chain=O(ident,H(ident,stop=CHAIN_STOP))
 cast=A(K('call'),F('function',names(['cast'])),F('arguments',H(A({'nthChild':2},chain))))
 return O(chain,cast,H(cast,stop=CHAIN_STOP))
def exact(values):return '(?i)^(?:'+'|'.join(re.escape(v) for v in sorted(values))+')$'
def method_call(methods,receiver=None):
 return A(K('call'),F('function',A(K('attribute'),F('attribute',names(methods)),*([F('object',receiver)] if receiver else []))))
def decorated(values):
 return I(A(K('decorated_definition'),H(A(K('decorator'),H(O(term(values),call(values)))))),'neighbor')
def files(regex=None,exclude=()):
 gate={}
 if regex:gate['regex']=regex
 if exclude:gate['notRegex']='^(?:api/)?(?:'+'|'.join(re.escape(v) for v in sorted(exclude))+')$'
 return {'filePath':gate} if gate else TRUE
def under(scope):return '^(?:api/)?'+re.escape(scope)+'(?:/|$)'

def legacy_clauses(m):
 triggers='|'.join(re.escape(v) for v in m.LEGACY_EXECUTION_SURFACES_EXTRACTOR.source_triggers)
 triggered=choice('oracleAnalyzer',TRUE,I(A(K('module'),RX(triggers))))
 internal={'filePath':{'regex':'^(?:api/)?(?:impl/|data_layer/|l[0-9])',
  'notRegex':'(?:^|/)tests/|^(?:api/)?(?:'+'|'.join(re.escape(v) for v in sorted([m.LEGACY_DELEGATE_CANONICAL_DEF_FILE,'l2050_transaction/execution_boundary.py']))+')$'}}
 legacy_fn=A(K('function_definition'),decorated(m.LEGACY_WRAPPER_DECORATORS|m.DIRECT_LEGACY_SURFACE_DECORATORS))
 outside_legacy=N(I(legacy_fn))
 plan=tokens(exact({'server_plan','plan','read_plan','write_plan'}))
 txn=tokens(exact({'txn','transaction','read_txn','write_txn','plan_txn','lazy_txn','scope','query','queries','lazy_query','typed_query','sql_query'}))
 cursor=tokens('(?i)cursor|^(?:db|conn|connection|raw_conn|raw_connection)$')
 entries=['scan_file_content']
 clauses=[]
 def local(id,rule,code):
  c=C(id,rule,code,entries);c['violation']['message']='Architecture contract violation';clauses.append(c)
 local('delegate-definition',A(fn('_legacy_delegate_stub'),files(exclude=m.LEGACY_DELEGATE_ALLOWLIST|{m.LEGACY_DELEGATE_CANONICAL_DEF_FILE})),'LEG-020')
 local('versioning-wrappers',A(K('function_definition'),F('name',names(m.VERSIONING_COMPAT_WRAPPERS)),files(exclude=m.VERSIONING_COMPAT_ALLOWLIST),triggered),'LEG-030')
 for scope,allowlist in sorted(m.RAW_TXN_SCOPE_ALLOWLISTS.items()):
  local('raw-txn-'+scope,A(method_call(m.RAW_TXN_METHODS),files(under(scope),allowlist)),'LEG-040')
 for scope,allowlist in sorted(m.RAW_CURSOR_SCOPE_ALLOWLISTS.items()):
  local('raw-cursor-'+scope,A(method_call(m.RAW_CURSOR_METHODS,cursor),files(under(scope),allowlist)),'LEG-050')
 local('eager-plan',A(method_call(m.EAGER_EXECUTION_METHODS,plan),internal,outside_legacy,triggered),'LEG-081')
 local('eager-txn',A(method_call(m.EAGER_EXECUTION_METHODS,A(N(plan),txn)),internal,outside_legacy,triggered),'LEG-080')
 local('boundary-calls',A(K('call'),F('function',names(m.LEGACY_EXECUTION_BOUNDARY_CALLS)),internal),'LEG-082')
 boundary_module=RX(r'^(?:l2050_transaction|l2000_connection)\.execution_boundary$')
 boundary_name=O(A(K('dotted_name'),RX('^legacy_execution_boundary$')),A(K('aliased_import'),F('name',RX('^legacy_execution_boundary$'))))
 local('raw-boundary-import',A(K('import_from_statement'),F('module_name',boundary_module),H(boundary_name),internal),'LEG-084')
 graph=files(under(m.GRAPH_FALLBACK_SCOPE),m.GRAPH_FALLBACK_ALLOWLIST)
 local('graph-fallback-none',A(O(K('if_statement'),K('elif_clause')),graph,F('condition',A(K('comparison_operator'),RX(r'\bis\s+None$'),H(A(N(K('none')),tokens(exact({'mixin','_plan_mixin','plan_mixin'})))))),triggered),'LEG-060')

 # Relational checks: candidate rows joined by span containment within each file.
 def row(rule,project,each=None):return E(A(gate(entries),triggered,rule),project,each)
 import_rule=A(K('import_from_statement'),P('IMPORT'),F('module_name',P('MODULE')),imported(O(A(K('dotted_name'),P('IMPORTED')),A(K('aliased_import'),F('name',P('IMPORTED')),F('alias',P('ALIAS'))))))
 inputs={
  'fns':row(A(K('function_definition'),F('name',P('NAME'))),{'name':'capture.NAME'}),
  'legacy_fns':row(A(legacy_fn,F('name',P('NAME'))),{'name':'capture.NAME'}),
  'wrapper_fns':row(A(K('function_definition'),decorated(m.LEGACY_WRAPPER_DECORATORS),F('name',P('NAME'))),{'name':'capture.NAME'}),
  'classes':row(A(K('class_definition'),F('name',P('NAME'))),{'name':'capture.NAME'}),
  'ident_calls':row(A(K('call'),F('function',A(K('identifier'),P('FN')))),{'callee':'capture.FN'}),
  'attr_calls':row(A(K('call'),F('function',A(K('attribute'),P('FN'),F('attribute',P('ATTR')),F('object',P('OBJ'))))),{'callee':'capture.ATTR','full_callee':'capture.FN','object_text':'capture.OBJ'}),
  'other_calls':row(A(K('call'),F('function',A(P('FN'),N(O(K('identifier'),K('attribute')))))),{'full_callee':'capture.FN'}),
  'from_imports':row(import_rule,{'module':'capture.MODULE','name':'capture.IMPORTED','alias':'capture.ALIAS'},each='IMPORTED'),
  'direct_imports':row(A(K('import_statement'),imported(O(A(K('dotted_name'),P('IMPORTED')),A(K('aliased_import'),F('name',P('IMPORTED')))))),{'name':'capture.IMPORTED'},each='IMPORTED'),
 }
 boundary_calls=literals(m.LEGACY_EXECUTION_BOUNDARY_CALLS)
 stages=[
 S('paths',"""SELECT DISTINCT __path,CASE WHEN substr(__path,1,4)='api/' THEN substr(__path,5) ELSE __path END AS path FROM (
 SELECT __path FROM fns UNION SELECT __path FROM ident_calls UNION SELECT __path FROM attr_calls UNION SELECT __path FROM from_imports UNION SELECT __path FROM direct_imports)""",('__path',)),
 S('calls',"""SELECT __path,__start,__end,__start||':'||__end AS id,'identifier' AS callee_kind,callee,callee AS full_callee,NULL AS object_text FROM ident_calls
 UNION ALL SELECT __path,__start,__end,__start||':'||__end,'attribute',callee,full_callee,object_text FROM attr_calls
 UNION ALL SELECT __path,__start,__end,__start||':'||__end,'other',full_callee,full_callee,NULL FROM other_calls""",('__path','__start')),
 S('function_symbols',"""SELECT f.__path,f.__start AS id,f.__start,f.__end,f.name,coalesce((SELECT group_concat(name,'.')||'.' FROM (SELECT c.name FROM classes c WHERE c.__path=f.__path AND c.__start<f.__start AND c.__end>=f.__end ORDER BY c.__start)),'')||f.name AS symbol,
 EXISTS(SELECT 1 FROM legacy_fns l WHERE l.__path=f.__path AND l.__start=f.__start) AS legacy,
 EXISTS(SELECT 1 FROM wrapper_fns w WHERE w.__path=f.__path AND w.__start=f.__start) AS wrapper FROM fns f""",('__path','id'),('__path','__start')),
 # A call's owner is its innermost enclosing function.
 S('owned_calls',"SELECT c.*,(SELECT f.id FROM function_symbols f WHERE f.__path=c.__path AND f.__start<c.__start AND f.__end>=c.__end ORDER BY f.__start DESC LIMIT 1) AS fn FROM calls c",('__path','fn'),('__path','id')),
 S('active_functions',"SELECT * FROM (SELECT *,row_number() OVER(PARTITION BY __path,symbol ORDER BY __start DESC) AS selected FROM function_symbols) WHERE selected=1",('__path','id'),('__path','symbol'),('__path','name')),
 S('valid_boundary',"""SELECT DISTINCT __path FROM wrapper_fns
 UNION SELECT DISTINCT f.__path FROM function_symbols f JOIN owned_calls c ON c.__path=f.__path AND c.fn=f.id AND c.callee_kind='identifier' AND c.callee IN """+boundary_calls+""" JOIN fns lazy ON lazy.__path=f.__path AND lazy.name=CASE WHEN substr(f.name,-6)='_async' THEN substr(f.name,1,length(f.name)-6)||'_lazy' ELSE f.name||'_lazy' END""",('__path',)),
 S('raw_boundaries',"SELECT * FROM from_imports WHERE module IN ('l2050_transaction.execution_boundary','l2000_connection.execution_boundary') AND name='legacy_execution_boundary'",('__path','alias')),
 S('eager_seeds',"SELECT f.__path,f.id AS fn,c.id AS anchor,c.__start,c.__end FROM active_functions f JOIN owned_calls c ON c.__path=f.__path AND c.fn=f.id WHERE NOT f.legacy AND c.callee_kind='attribute' AND c.callee IN ('resolve','resolve_async')",('__path','fn')),
 S('call_candidates',"""SELECT f.__path,f.id AS caller,c.id AS anchor,c.__start,c.__end,c.callee AS candidate FROM active_functions f JOIN owned_calls c ON c.__path=f.__path AND c.fn=f.id WHERE NOT f.legacy AND c.callee NOT IN ('resolve','resolve_async','_legacy_delegate_stub','legacy_execution_boundary') AND c.callee NOT IN """+boundary_calls+"""
 UNION SELECT f.__path,f.id,c.id,c.__start,c.__end,substr(f.symbol,1,length(f.symbol)-length(f.name))||c.callee FROM active_functions f JOIN owned_calls c ON c.__path=f.__path AND c.fn=f.id WHERE NOT f.legacy AND c.callee_kind='attribute' AND c.callee NOT IN ('resolve','resolve_async') AND c.object_text IN ('self','cls') AND instr(f.symbol,'.')>0""",('__path','candidate')),
 S('resolved_calls',"SELECT c.__path,c.caller,c.anchor,c.__start,c.__end,min(f.id) AS callee FROM call_candidates c JOIN active_functions f ON f.__path=c.__path AND (f.name=c.candidate OR f.symbol=c.candidate) GROUP BY c.__path,c.caller,c.anchor,c.__start,c.__end,c.candidate HAVING count(*)=1 AND min(f.id)!=c.caller",('__path','callee')),
 S('eager_reach',"WITH RECURSIVE reaches(__path,fn) AS (SELECT __path,fn FROM eager_seeds UNION SELECT r.__path,c.caller FROM reaches r JOIN resolved_calls c ON c.__path=r.__path AND c.callee=r.fn JOIN active_functions f ON f.__path=c.__path AND f.id=c.caller WHERE NOT f.legacy) SELECT * FROM reaches",('__path','fn')),
 S('eager_anchors',"""SELECT __path,fn,anchor,__start,__end FROM (
 SELECT e.__path,e.fn,e.anchor,e.__start,e.__end,row_number() OVER(PARTITION BY e.__path,e.fn ORDER BY e.__start,e.__end DESC) AS selected FROM eager_seeds e) WHERE selected=1
 UNION SELECT __path,fn,anchor,__start,__end FROM (
 SELECT c.__path,c.caller AS fn,c.anchor,c.__start,c.__end,row_number() OVER(PARTITION BY c.__path,c.caller ORDER BY c.__start,c.__end DESC) AS selected FROM resolved_calls c JOIN eager_reach r ON r.__path=c.__path AND r.fn=c.callee WHERE NOT EXISTS(SELECT 1 FROM eager_seeds s WHERE s.__path=c.__path AND s.fn=c.caller)) WHERE selected=1""",('__path','anchor')),
 ]
 rows=[]
 def add(table,condition,code,identity='n.id'):
  rows.append(f"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,{q(code)} AS issue,{identity} AS identity FROM {table} n JOIN paths p ON p.__path=n.__path WHERE {condition}")
 internal_sql="rx_matches(p.path,'^(?:impl/|data_layer/|l[0-9])') AND NOT rx_matches(p.path,'(?:^|/)tests/') AND p.path NOT IN "+literals([m.LEGACY_DELEGATE_CANONICAL_DEF_FILE,'l2050_transaction/execution_boundary.py'])
 valid="EXISTS(SELECT 1 FROM valid_boundary v WHERE v.__path=n.__path)"
 add('owned_calls',"n.callee_kind='identifier' AND n.callee='_legacy_delegate_stub' AND p.path NOT IN "+literals(m.LEGACY_DELEGATE_ALLOWLIST)+" AND NOT ("+valid+" AND EXISTS(SELECT 1 FROM function_symbols f WHERE f.__path=n.__path AND f.id=n.fn AND f.wrapper AND NOT EXISTS(SELECT 1 FROM owned_calls other WHERE other.__path=f.__path AND other.fn=f.id AND other.full_callee!='_legacy_delegate_stub')))",'LEG-021')
 add('eager_anchors',internal_sql+" AND EXISTS(SELECT 1 FROM active_functions f WHERE f.__path=n.__path AND f.id=n.fn AND NOT f.legacy AND p.path||':'||f.symbol NOT IN "+literals(m.ADMIN_EAGER_TERMINAL_ALLOWLIST)+')','LEG-083','n.anchor')
 add('calls',internal_sql+" AND n.callee_kind='identifier' AND (n.callee='legacy_execution_boundary' OR EXISTS(SELECT 1 FROM raw_boundaries r WHERE r.__path=n.__path AND r.__start<n.__start AND n.callee=coalesce(r.alias,r.name)))",'LEG-085')
 # Import inventory: direct import aliases each count; a from import counts once.
 import_condition="p.path NOT IN "+literals(m.LEGACY_WRAPPER_IMPORT_ALLOWLIST)+' AND NOT '+valid
 add('from_imports',import_condition+" AND (n.module IN "+literals(m.LEGACY_WRAPPERS_MODULE_ALIASES)+" OR n.module='l2050_transaction' AND n.name='legacy_wrappers') AND NOT (rx_matches(p.path,'(?:^|/)tests/') AND NOT EXISTS(SELECT 1 FROM from_imports i WHERE i.__path=n.__path AND i.module IN "+literals(m.LEGACY_WRAPPERS_MODULE_ALIASES)+" AND i.name!='validate_legacy_delegates'))",'LEG-010',"n.__start||':'||n.__end")
 add('direct_imports',import_condition+" AND n.name IN "+literals(m.LEGACY_WRAPPERS_MODULE_ALIASES),'LEG-010','n.name')
 clauses.append({'id':'legacy-execution-graphs','relational':{'inputs':inputs,'stages':stages,'query':' UNION ALL '.join(rows),'key':['issue','identity'],'code':'issue','partitionBy':'file'},
  'violation':{'code':'architecture','message':'Architecture contract violation','anchor':'witness.span'}})
 # Upstream tests each source line for these tokens; in syntax they are identifier text, so comments,
 # docstrings and string literals no longer count.
 def within(scopes):return '^(?:api/)?(?:'+'|'.join(re.escape(s) for s in sorted(scopes))+')/'
 for token in sorted(m.SQL_POST_FETCH_FILTER_TOKENS):
  c=C('post-fetch-'+token,A(K('identifier'),RX(re.escape(token)),{'filePath':{'regex':within(m.SQL_POST_FETCH_FILTER_SCOPES),'notRegex':'^docs/data_history/'}}),'LEG-071',['scan_sql_post_fetch_filter_content'])
  c['violation']['message']='SQL-facing post-fetch filter';clauses.append(c)
 fallback=['grammar_rows_lazy','grammar_row_lazy','fetchone_via_plan','fetchall_via_plan']
 c=C('graph-fallback-text',A(K('identifier'),RX('|'.join(re.escape(v) for v in fallback)),
  {'filePath':{'regex':within([m.GRAPH_FALLBACK_SCOPE]),'notRegex':'^(?:api/)?(?:'+'|'.join(re.escape(f) for f in sorted(m.GRAPH_FALLBACK_ALLOWLIST))+')$'}}),'LEG-061',['scan_file_content'])
 c['violation']['message']='Fallback grammar bridge';clauses.append(c)
 for basename,allowlist in sorted(m.EXACT_FILE_NAME_ALLOWLISTS.items()):
  clauses.append({'id':'inventory-'+basename,'forbid':{'regex':{'source':'file.path','pattern':choice('oracleAnalyzer',r'\b\B',r'(?:^|/)'+re.escape(basename)+'$'),'paths':{'include':['**/*'],'exclude':sorted(allowlist)}}},'violation':{'code':'LEG-001','message':'Legacy file outside inventory','anchor':'witness.span'}})
 clauses.append({'id':'removed-read-leaves','forbid':{'regex':{'source':'file.path','pattern':choice('oracleAnalyzer',r'\b\B','^(?:'+'|'.join(re.escape(v) for v in sorted(m.REMOVED_SQL_READ_LEAF_FILES))+')$')}},'violation':{'code':'LEG-070','message':'Removed SQL read leaf','anchor':'witness.span'}})
 return clauses
