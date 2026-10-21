# SPDX-License-Identifier: Apache-2.0
"""Legacy execution inventories, receiver tokens, and eager-call reachability."""
from .common import *
from .ast_relations import relational,python_stages

def legacy_clauses(m):
 stages=python_stages()+[
 S('paths',"SELECT DISTINCT __path,CASE WHEN substr(__path,1,4)='api/' THEN substr(__path,5) ELSE __path END AS path FROM nodes",('__path',)),
 S('decorators',"SELECT f.__path,f.id AS fn,d.id,coalesce(c.callee,a.attr,child.text) AS name FROM functions f JOIN nodes wrapper ON wrapper.__path=f.__path AND wrapper.id=f.parent AND wrapper.kind='decorated_definition' JOIN nodes d ON d.__path=wrapper.__path AND d.parent=wrapper.id AND d.kind='decorator' JOIN nodes child ON child.__path=d.__path AND child.parent=d.id LEFT JOIN calls c ON c.__path=child.__path AND c.id=child.id LEFT JOIN attributes a ON a.__path=child.__path AND a.id=child.id",('__path','fn','name')),
 S('function_symbols',"""SELECT * FROM (SELECT f.*,coalesce((SELECT group_concat(name,'.')||'.' FROM (SELECT c.name FROM classes c WHERE c.__path=f.__path AND c.__start<f.__start AND c.__end>=f.__end ORDER BY c.__start)),'')||f.name AS symbol,
 EXISTS(SELECT 1 FROM decorators d WHERE d.__path=f.__path AND d.fn=f.id AND d.name IN """+literals(m.LEGACY_WRAPPER_DECORATORS|m.DIRECT_LEGACY_SURFACE_DECORATORS)+""") AS legacy FROM functions f)""",('__path','id'),('__path','symbol')),
 S('active_functions',"SELECT * FROM (SELECT *,row_number() OVER(PARTITION BY __path,symbol ORDER BY __start DESC) AS selected FROM function_symbols) WHERE selected=1",('__path','id'),('__path','symbol'),('__path','name')),
 S('valid_boundary',"""SELECT DISTINCT f.__path FROM functions f JOIN decorators d ON d.__path=f.__path AND d.fn=f.id WHERE d.name IN """+literals(m.LEGACY_WRAPPER_DECORATORS)+"""
 UNION SELECT DISTINCT f.__path FROM functions f JOIN ownership o ON o.__path=f.__path AND o.fn=f.id JOIN calls c ON c.__path=o.__path AND c.id=o.id AND c.callee_kind='identifier' AND c.callee IN """+literals(m.LEGACY_EXECUTION_BOUNDARY_CALLS)+""" JOIN functions lazy ON lazy.__path=f.__path AND lazy.name=CASE WHEN substr(f.name,-6)='_async' THEN substr(f.name,1,length(f.name)-6)||'_lazy' ELSE f.name||'_lazy' END""",('__path',)),
 S('receiver_edges',"""SELECT a.__path,a.object AS src,a.id AS dst FROM attributes a
 UNION SELECT a.__path,a.attribute_kind||':'||a.attribute_start||':'||a.attribute_end,a.id FROM attributes a
 UNION SELECT c.__path,c.callee_id,c.id FROM calls c WHERE c.callee!='cast' OR (SELECT count(*) FROM arguments a WHERE a.__path=c.__path AND a.call=c.id)<2
 UNION SELECT c.__path,a.id,c.id FROM calls c JOIN arguments a ON a.__path=c.__path AND a.call=c.id AND a.position=1 WHERE c.callee='cast'
 UNION SELECT n.__path,c.id,n.id FROM nodes n JOIN nodes c ON c.__path=n.__path AND c.parent=n.id AND c.ordinal=0 WHERE n.kind IN ('subscript','parenthesized_expression')""",('__path','src')),
 S('receiver_tokens',"WITH RECURSIVE tokens(__path,id,token) AS (SELECT __path,id,lower(text) FROM nodes WHERE kind='identifier' UNION SELECT t.__path,e.dst,t.token FROM tokens t JOIN receiver_edges e ON e.__path=t.__path AND e.src=t.id) SELECT * FROM tokens",('__path','id','token')),
 S('legacy_imports',"""SELECT n.*,module.text AS module FROM nodes n JOIN nodes module ON module.__path=n.__path AND module.id=n.name_kind||':'||n.name_start||':'||n.name_end WHERE 0
 UNION SELECT n.*,module.text FROM nodes n JOIN nodes module ON module.__path=n.__path AND module.parent=n.id AND module.ordinal=0 WHERE n.kind='import_from_statement'""",('__path','id')),
 ]
 # Import fields have dedicated grammar roles; extract these directly to preserve alias multiplicity.
 import_rule=A(K('import_from_statement'),P('IMPORT'),F('module_name',P('MODULE')),F('name',O(A(K('dotted_name'),P('IMPORTED')),A(K('aliased_import'),F('name',P('IMPORTED')),F('alias',P('ALIAS'))))))
 extra={'from_imports':E(import_rule,{'module':'capture.MODULE','name':'capture.IMPORTED','alias':'capture.ALIAS','line':'capture.IMPORT.startLine'},each='IMPORTED'),
 'direct_imports':E(A(K('import_statement'),F('name',O(A(K('dotted_name'),P('IMPORTED')),A(K('aliased_import'),F('name',P('IMPORTED')))))) ,{'name':'capture.IMPORTED'},each='IMPORTED')}

 stages.extend([
 S('raw_boundaries',"SELECT * FROM from_imports WHERE module IN ('l2050_transaction.execution_boundary','l2000_connection.execution_boundary') AND name='legacy_execution_boundary'",('__path','alias')),
 S('eager_seeds',"SELECT f.__path,f.id AS fn,c.id AS anchor FROM active_functions f JOIN ownership o ON o.__path=f.__path AND o.fn=f.id JOIN calls c ON c.__path=o.__path AND c.id=o.id WHERE NOT f.legacy AND c.callee_kind='attribute' AND c.callee IN ('resolve','resolve_async')",('__path','fn')),
 S('call_candidates',"""SELECT f.__path,f.id AS caller,c.id AS anchor,c.callee AS candidate FROM active_functions f JOIN ownership o ON o.__path=f.__path AND o.fn=f.id JOIN calls c ON c.__path=o.__path AND c.id=o.id WHERE NOT f.legacy AND c.callee NOT IN ('resolve','resolve_async','_legacy_delegate_stub','legacy_execution_boundary') AND c.callee NOT IN """+literals(m.LEGACY_EXECUTION_BOUNDARY_CALLS)+"""
 UNION SELECT f.__path,f.id,c.id,substr(f.symbol,1,length(f.symbol)-length(f.name))||c.callee FROM active_functions f JOIN ownership o ON o.__path=f.__path AND o.fn=f.id JOIN calls c ON c.__path=o.__path AND c.id=o.id JOIN attributes a ON a.__path=c.__path AND a.id=c.callee_id WHERE NOT f.legacy AND c.callee NOT IN ('resolve','resolve_async') AND a.object_text IN ('self','cls') AND instr(f.symbol,'.')>0""",('__path','candidate')),
 S('resolved_calls',"SELECT c.__path,c.caller,c.anchor,min(f.id) AS callee FROM call_candidates c JOIN active_functions f ON f.__path=c.__path AND (f.name=c.candidate OR f.symbol=c.candidate) GROUP BY c.__path,c.caller,c.anchor,c.candidate HAVING count(*)=1 AND min(f.id)!=c.caller",('__path','callee')),
 S('eager_reach',"WITH RECURSIVE reaches(__path,fn) AS (SELECT __path,fn FROM eager_seeds UNION SELECT r.__path,c.caller FROM reaches r JOIN resolved_calls c ON c.__path=r.__path AND c.callee=r.fn JOIN active_functions f ON f.__path=c.__path AND f.id=c.caller WHERE NOT f.legacy) SELECT * FROM reaches",('__path','fn')),
 S('eager_anchors',"""SELECT __path,fn,anchor FROM (
 SELECT e.__path,e.fn,e.anchor,n.__start,row_number() OVER(PARTITION BY e.__path,e.fn ORDER BY n.__start,n.__end DESC) AS selected FROM eager_seeds e JOIN nodes n ON n.__path=e.__path AND n.id=e.anchor) WHERE selected=1
 UNION SELECT __path,fn,anchor FROM (
 SELECT c.__path,c.caller AS fn,c.anchor,n.__start,row_number() OVER(PARTITION BY c.__path,c.caller ORDER BY n.__start,n.__end DESC) AS selected FROM resolved_calls c JOIN eager_reach r ON r.__path=c.__path AND r.fn=c.callee JOIN nodes n ON n.__path=c.__path AND n.id=c.anchor WHERE NOT EXISTS(SELECT 1 FROM eager_seeds s WHERE s.__path=c.__path AND s.fn=c.caller)) WHERE selected=1""",('__path','anchor')),
 ])
 rows=[]
 def add(table,condition,code,identity='n.id'):
  rows.append(f"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,{q(code)} AS issue,{identity} AS identity FROM {table} n JOIN paths p ON p.__path=n.__path WHERE {condition}")
 internal="rx_matches(p.path,'^(?:impl/|data_layer/|l[0-9])') AND NOT rx_matches(p.path,'(?:^|/)tests/') AND p.path NOT IN "+literals([m.LEGACY_DELEGATE_CANONICAL_DEF_FILE,'l2050_transaction/execution_boundary.py'])
 valid="EXISTS(SELECT 1 FROM valid_boundary v WHERE v.__path=n.__path)"
 wrapper_decorator="EXISTS(SELECT 1 FROM decorators d WHERE d.__path=f.__path AND d.fn=f.id AND d.name IN "+literals(m.LEGACY_WRAPPER_DECORATORS)+')'
 add('calls',"n.callee_kind='identifier' AND n.callee='_legacy_delegate_stub' AND p.path NOT IN "+literals(m.LEGACY_DELEGATE_ALLOWLIST)+" AND NOT ("+valid+" AND EXISTS(SELECT 1 FROM ownership o JOIN function_symbols f ON f.__path=o.__path AND f.id=o.fn WHERE o.__path=n.__path AND o.id=n.id AND "+wrapper_decorator+" AND NOT EXISTS(SELECT 1 FROM ownership own JOIN calls other ON other.__path=own.__path AND other.id=own.id WHERE own.__path=f.__path AND own.fn=f.id AND other.full_callee!='_legacy_delegate_stub')))",'LEG-021')
 add('functions',"n.name='_legacy_delegate_stub' AND p.path NOT IN "+literals(m.LEGACY_DELEGATE_ALLOWLIST|{m.LEGACY_DELEGATE_CANONICAL_DEF_FILE}),'LEG-020')
 add('functions','n.name IN '+literals(m.VERSIONING_COMPAT_WRAPPERS)+' AND p.path NOT IN '+literals(m.VERSIONING_COMPAT_ALLOWLIST),'LEG-030')
 for scope,allowlist in m.RAW_TXN_SCOPE_ALLOWLISTS.items():
  add('calls',"n.callee_kind='attribute' AND n.callee IN "+literals(m.RAW_TXN_METHODS)+" AND (p.path="+q(scope)+" OR substr(p.path,1,"+str(len(scope)+1)+")="+q(scope+'/')+") AND p.path NOT IN "+literals(allowlist),'LEG-040',q(scope)+"||n.id")
 for scope,allowlist in m.RAW_CURSOR_SCOPE_ALLOWLISTS.items():
  add('calls',"n.callee_kind='attribute' AND n.callee IN "+literals(m.RAW_CURSOR_METHODS)+" AND (p.path="+q(scope)+" OR substr(p.path,1,"+str(len(scope)+1)+")="+q(scope+'/')+") AND p.path NOT IN "+literals(allowlist)+" AND EXISTS(SELECT 1 FROM receiver_tokens t WHERE t.__path=n.__path AND t.id=n.receiver AND (instr(t.token,'cursor')>0 OR t.token IN ('db','conn','connection','raw_conn','raw_connection')))",'LEG-050',q(scope)+"||n.id")
 no_legacy="NOT EXISTS(SELECT 1 FROM function_symbols f WHERE f.__path=n.__path AND f.__start<=n.__start AND f.__end>=n.__end AND f.legacy)"
 plan="EXISTS(SELECT 1 FROM receiver_tokens t WHERE t.__path=n.__path AND t.id=n.receiver AND t.token IN ('server_plan','plan','read_plan','write_plan'))"
 txn="EXISTS(SELECT 1 FROM receiver_tokens t WHERE t.__path=n.__path AND t.id=n.receiver AND t.token IN ('txn','transaction','read_txn','write_txn','plan_txn','lazy_txn','scope','query','queries','lazy_query','typed_query','sql_query'))"
 add('calls',internal+" AND n.callee_kind='attribute' AND n.callee IN "+literals(m.EAGER_EXECUTION_METHODS)+' AND '+no_legacy+' AND '+plan,'LEG-081')
 add('calls',internal+" AND n.callee_kind='attribute' AND n.callee IN "+literals(m.EAGER_EXECUTION_METHODS)+' AND '+no_legacy+' AND NOT '+plan+' AND '+txn,'LEG-080')
 add('calls',internal+" AND n.callee_kind='identifier' AND n.callee IN "+literals(m.LEGACY_EXECUTION_BOUNDARY_CALLS),'LEG-082')
 add('nodes',internal+" AND EXISTS(SELECT 1 FROM eager_anchors e JOIN active_functions f ON f.__path=e.__path AND f.id=e.fn WHERE e.__path=n.__path AND e.anchor=n.id AND NOT f.legacy AND p.path||':'||f.symbol NOT IN "+literals(m.ADMIN_EAGER_TERMINAL_ALLOWLIST)+')','LEG-083')
 add('nodes',internal+" AND EXISTS(SELECT 1 FROM raw_boundaries r WHERE r.__path=n.__path AND r.__start=n.__start AND r.__end=n.__end)",'LEG-084')
 add('calls',internal+" AND n.callee_kind='identifier' AND (n.callee='legacy_execution_boundary' OR EXISTS(SELECT 1 FROM raw_boundaries r WHERE r.__path=n.__path AND r.__start<n.__start AND n.callee=coalesce(r.alias,r.name)))",'LEG-085')
 # Import inventory: direct import aliases each count; a from import counts once.
 import_condition="p.path NOT IN "+literals(m.LEGACY_WRAPPER_IMPORT_ALLOWLIST)+' AND NOT '+valid
 add('nodes',"n.kind='import_from_statement' AND "+import_condition+" AND EXISTS(SELECT 1 FROM from_imports i WHERE i.__path=n.__path AND i.__start=n.__start AND i.__end=n.__end AND (i.module IN "+literals(m.LEGACY_WRAPPERS_MODULE_ALIASES)+" OR i.module='l2050_transaction' AND i.name='legacy_wrappers')) AND NOT (rx_matches(p.path,'(?:^|/)tests/') AND NOT EXISTS(SELECT 1 FROM from_imports i WHERE i.__path=n.__path AND i.module IN "+literals(m.LEGACY_WRAPPERS_MODULE_ALIASES)+" AND i.name!='validate_legacy_delegates'))",'LEG-010')
 add('direct_imports',import_condition+" AND n.name IN "+literals(m.LEGACY_WRAPPERS_MODULE_ALIASES),'LEG-010','n.name')
 graph_scope="(p.path="+q(m.GRAPH_FALLBACK_SCOPE)+" OR substr(p.path,1,"+str(len(m.GRAPH_FALLBACK_SCOPE)+1)+")="+q(m.GRAPH_FALLBACK_SCOPE+'/')+") AND p.path NOT IN "+literals(m.GRAPH_FALLBACK_ALLOWLIST)
 add('nodes',graph_scope+" AND n.kind IN ('if_statement','elif_clause') AND EXISTS(SELECT 1 FROM nodes test WHERE test.__path=n.__path AND test.id=n.condition_kind||':'||n.condition_start||':'||n.condition_end AND test.kind='comparison_operator' AND rx_matches(test.text,'\\bis\\s+None$') AND (SELECT count(*) FROM nodes child WHERE child.__path=test.__path AND child.parent=test.id AND child.kind!='comment')=2 AND EXISTS(SELECT 1 FROM nodes lhs JOIN receiver_tokens t ON t.__path=lhs.__path AND t.id=lhs.id WHERE lhs.__path=test.__path AND lhs.parent=test.id AND lhs.ordinal=0 AND t.token IN ('mixin','_plan_mixin','plan_mixin')))",'LEG-060')
 query=' UNION ALL '.join(rows)
 result=relational('legacy-execution-graphs',stages,query)
 result['relational']['inputs'].update(extra)
 for input_ in result['relational']['inputs'].values():input_['evidence']['query']['ast']['rule']=A(gate(['scan_file_content']),choice('oracleAnalyzer',TRUE,O(A(K('module'),RX('|'.join(re.escape(v) for v in m.LEGACY_EXECUTION_SURFACES_EXTRACTOR.source_triggers))),I(A(K('module'),RX('|'.join(re.escape(v) for v in m.LEGACY_EXECUTION_SURFACES_EXTRACTOR.source_triggers)))))),input_['evidence']['query']['ast']['rule'])
 # Drop generic tables not read by this policy to preserve the stage bound.
 all_stages=result['relational']['stages'];by_name={s['name']:s for s in all_stages};needed=set();pending=[query]
 while pending:
  for name in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',pending.pop()):
   if name in by_name and name not in needed:needed.add(name);pending.append(by_name[name]['query'])
 result['relational']['stages']=[s for s in all_stages if s['name'] in needed]
 clauses=[result]
 for token in sorted(m.SQL_POST_FETCH_FILTER_TOKENS):
  clauses.append({'id':'post-fetch-'+token,'forbid':{'regex':{'source':'file.content',
   'pattern':{'$option':'entryPoint','cases':{e:re.escape(token) for e in ['all','scan_sql_post_fetch_filter_content']},'default':r'\b\B'},
   'lines':{'first':True,'terminators':'python'},'paths':{'include':[prefix+scope+'/**' for scope in sorted(m.SQL_POST_FETCH_FILTER_SCOPES) for prefix in ['','api/']],'exclude':['docs/data_history/**']}}},
   'violation':{'code':'LEG-071','message':'SQL-facing post-fetch filter','anchor':'witness.span'}})
 clauses.append({'id':'graph-fallback-text','forbid':{'regex':{'source':'file.content',
  'pattern':{'$option':'entryPoint','cases':{e:'grammar_rows_lazy|grammar_row_lazy|fetchone_via_plan|fetchall_via_plan' for e in ['all','scan_file_content']},'default':r'\b\B'},
  'lines':{'first':True,'terminators':'python'},'paths':{'include':[p+m.GRAPH_FALLBACK_SCOPE+'/**' for p in ['','api/']],'exclude':[p+f for p in ['','api/'] for f in m.GRAPH_FALLBACK_ALLOWLIST]}}},
  'violation':{'code':'LEG-061','message':'Fallback grammar bridge','anchor':'witness.span'}})
 for basename,allowlist in sorted(m.EXACT_FILE_NAME_ALLOWLISTS.items()):
  clauses.append({'id':'inventory-'+basename,'forbid':{'regex':{'source':'file.path','pattern':choice('oracleAnalyzer',r'\b\B',r'(?:^|/)'+re.escape(basename)+'$'),'paths':{'include':['**/*'],'exclude':sorted(allowlist)}}},'violation':{'code':'LEG-001','message':'Legacy file outside inventory','anchor':'witness.span'}})
 clauses.append({'id':'removed-read-leaves','forbid':{'regex':{'source':'file.path','pattern':choice('oracleAnalyzer',r'\b\B','^(?:'+'|'.join(re.escape(v) for v in sorted(m.REMOVED_SQL_READ_LEAF_FILES))+')$')}},'violation':{'code':'LEG-070','message':'Removed SQL read leaf','anchor':'witness.span'}})
 return clauses
