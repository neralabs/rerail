# SPDX-License-Identifier: Apache-2.0
"""Source-authored relational candidates for the shared symbol graph surveys."""
from __future__ import annotations
import re

def A(*rules):return {'all':list(rules)}
def O(*rules):return {'any':list(rules)}
def N(rule):return {'not':rule}
def K(kind):return {'kind':kind}
def P(name):return {'pattern':'$'+name}
def H(rule,field=None,stop='neighbor'):return {'has':{**rule,'stopBy':stop,**({'field':field} if field else {})}}
def I(rule,stop='end'):return {'inside':{**rule,'stopBy':stop}}
def F(field,rule):return H(rule,field)
def optional(rule):return O(rule,P('_'))
def E(rule,project,each=None,transform=None):
 query={'language':'python','syntax':'valid','rule':rule}
 if transform:query['transform']=transform
 return {'evidence':{'as':'row','query':{'ast':query},'project':project,**({'each':each} if each else {})}}
def S(name,query,*indexes):return {'name':name,'query':query,**({'indexes':[list(fields) for fields in indexes]} if indexes else {})}
def q(value):return "'"+str(value).replace("'","''")+"'"
def literals(values):return '('+','.join(q(value) for value in sorted(values))+')'
def regex(values):return '(?:'+'|'.join(re.escape(value) for value in sorted(values))+')'
MOD="replace(rx_replace(__path, '/__init__\\.py$|\\.py$', ''), '/', '.')"


def symbol_inputs():
 fn=A(K('function_definition'),N(I(K('function_definition'))),P('OWNER'),F('name',P('FNAME')),
      optional(I(A(K('class_definition'),F('name',P('CLASS')),P('CLASS_NODE')),K('class_definition'))),
      optional(F('return_type',P('RET'))),F('body',P('BODY')))
 normal=N(O(K('list_splat_pattern'),K('dictionary_splat_pattern'),A(K('typed_parameter'),H(O(K('list_splat_pattern'),K('dictionary_splat_pattern'))))))
 param=A(O(A(K('identifier'),P('NAME')),A(K('typed_parameter'),H(A(K('identifier'),P('NAME')))),
           A(O(K('default_parameter'),K('typed_default_parameter')),F('name',P('NAME')))),normal,
         N({'precedes':{**K('positional_separator'),'stopBy':'end'}}),I(K('parameters'),'neighbor'),I(fn,K('function_definition')),P('PARAM'))
 common={'name':'capture.FNAME','class_name':'capture.CLASS','return_annotation':'capture.RET','class_start':'capture.CLASS_NODE.startByte','body_start':'capture.BODY.startByte'}
 decl=O(K('function_definition'),K('class_definition'))
 all_assignment=A(K('assignment'),F('left',{'pattern':'__all__'}),P('ALL'),I(K('expression_statement'),'neighbor'),
                  I(K('module')),N(I(O(K('function_definition'),K('class_definition'),K('if_statement'),K('try_statement')))))
 inputs={
  'raw_functions':E(fn,common),
  'raw_classes':E(A(K('class_definition'),F('name',P('NAME')),F('body',P('BODY'))),{'name':'capture.NAME','body_start':'capture.BODY.startByte'}),
  'raw_decorators':E(A(K('decorator'),P('DECO'),I(A(K('decorated_definition'),H(A(decl,P('DECL')))),'neighbor')),
      {'owner':'capture.DECL.startByte','name':{'source':'capture.DECO','regex':r'^@\s*([^\s(]+)','group':1}}),
  'raw_parameters':E(A(param,optional(F('type',P('TYPE')))),
      {'owner':'capture.OWNER.startByte','name':'capture.NAME','annotation':'capture.TYPE'}),
  'raw_parameter_annotations':E(A(K('identifier'),P('TOKEN'),{'inside':{**param,'field':'type','stopBy':'end'}}),
      {'owner':'capture.OWNER.startByte','parameter':'capture.PARAM.startByte','name':'capture.TOKEN'}),
  'raw_return_annotations':E(A(K('identifier'),P('TOKEN'),{'inside':{**fn,'field':'return_type','stopBy':'end'}}),
      {'owner':'capture.OWNER.startByte','name':'capture.TOKEN'}),
  'raw_imports':E(A(O(K('import_from_statement'),K('import_statement')),
      {'regex':r'(?s)^(?:from\s+(?P<BASE>[^\s]+)\s+)?import\s+(?P<ITEMS>.*)$'},optional(A(I(K('module'),'neighbor'),P('TOP')))),
      {'base':'capture.BASE','items':'capture.ITEMS','top':'capture.TOP.startByte'}),
  'raw_bases':E(A(O(K('identifier'),K('attribute'),K('generic_type'),K('subscript')),P('BASE'),
      I(A(K('argument_list'),I(A(K('class_definition'),P('CLASS')),'neighbor')),'neighbor')),
      {'owner':'capture.CLASS.startByte','name':'capture.BASE'}),
  'raw_all':E(A(all_assignment,optional(F('right',A(O(K('list'),K('tuple')),P('ARRAY'))))),{'array':'capture.ARRAY'}),
  'raw_all_names':E(A(O(K('string'),K('concatenated_string')),P('VALUE'),
      I(A(O(K('list'),K('tuple')),I(all_assignment,'neighbor')),'neighbor')),
      {'owner':'capture.ALL.startByte','name':'capture.TEXT.text'},transform={'TEXT':{'replace':{'source':'$VALUE','value':True,'replace':r'\b\B','by':''}}}),
 }
 for key in ('raw_all','raw_all_names'):
  inputs[key]['evidence']['query']['ast']['file']={'path':{'regex':r'^data_layer/[^/]+\.py$'}}
 return inputs


def symbol_stages(graph):
 # These tables contain source facts, never expected diagnostics. A declaration
 # has the same ID after a reassignment; a repeated declaration keeps its final body.
 stages=[
 S('inventory',f"""SELECT *, module || '::' || CASE WHEN class_name IS NULL THEN name ELSE class_name || '.' || name END AS id,
 module || '.' || CASE WHEN class_name IS NULL THEN name ELSE class_name || '.' || name END AS fqname
 FROM (SELECT __path,__start,__end,name,class_name,return_annotation,class_start,body_start, {MOD} AS module, CASE WHEN class_name IS NULL THEN 'function' ELSE 'method' END AS kind FROM raw_functions
 UNION ALL SELECT __path,__start,__end,name,NULL,NULL,NULL,body_start,{MOD},'class' FROM raw_classes)""",('id',),('module','name')),
 S('declarations',"""SELECT * FROM (SELECT *,row_number() OVER (PARTITION BY id ORDER BY __start DESC) AS selected FROM inventory) WHERE selected=1""",('id',),('__path','__start'),('module','name'),('fqname',),('kind','name'),('module','class_name')),
 S('decorators',"SELECT * FROM raw_decorators",('__path','owner')),
 S('parameters',"""SELECT p.*, d.id AS symbol FROM raw_parameters p JOIN declarations d ON d.__path=p.__path AND d.__start=p.owner
 WHERE d.class_name IS NULL OR p.name NOT IN ('self','cls')""",('symbol',),('__path','owner'),('__path','__start')),
 S('annotation_names',"""SELECT d.id AS symbol,a.name FROM raw_return_annotations a JOIN declarations d ON d.__path=a.__path AND d.__start=a.owner
 UNION SELECT p.symbol,a.name FROM raw_parameter_annotations a JOIN parameters p ON p.__path=a.__path AND p.__start=a.parameter""",('symbol','name')),
 S('imports',f"""WITH RECURSIVE
 tokens(path,ordinal,top,base,item,rest) AS (
 SELECT __path,__start,top,base,'',rx_replace(items,'#[^\\n]*','') || ',' FROM raw_imports
 UNION ALL SELECT path,ordinal,top,base,trim(substr(rest,1,instr(rest,',')-1),' ()' || char(10) || char(13)),substr(rest,instr(rest,',')+1) FROM tokens WHERE rest<>''),
 parts AS (SELECT *,rx_extract(item,'^(\\S+)',1) AS original, rx_extract(item,'\\s+as\\s+(\\S+)$',1) AS alias,
 replace(rx_replace(path,'/__init__\\.py$|\\.py$',''),'/','.') AS module FROM tokens WHERE item<>''),
 relative(path,ordinal,top,base,original,alias,module,prefix,remaining) AS (
 SELECT path,ordinal,top,base,original,alias,module,CASE WHEN path LIKE '%/__init__.py' THEN module ELSE rx_replace(module,'\\.?[^.]+$','') END,
 coalesce(length(base)-length(ltrim(base,'.'))-1,-1) FROM parts
 UNION ALL SELECT path,ordinal,top,base,original,alias,module,rx_replace(prefix,'\\.?[^.]+$',''),remaining-1 FROM relative WHERE remaining>0),
 resolved AS (SELECT *,CASE WHEN base IS NULL THEN original WHEN base NOT LIKE '.%' THEN base || '.' || original
 ELSE CASE WHEN prefix='' THEN '' ELSE prefix || '.' END || CASE WHEN ltrim(base,'.')='' THEN '' ELSE ltrim(base,'.') || '.' END || original END AS target,
 coalesce(alias,CASE WHEN base IS NULL THEN rx_extract(original,'^[^.]+',0) ELSE original END) AS name FROM relative WHERE remaining<=0)
 SELECT path,ordinal,top,module,name,target,CASE WHEN base IS NULL THEN 'module' ELSE 'symbol' END AS kind
 FROM (SELECT *,row_number() OVER (PARTITION BY module,name,base IS NULL ORDER BY ordinal DESC) AS selected FROM resolved WHERE original<>'*') WHERE selected=1""",('module','name'),('target',)),
 S('resolved_names',"""WITH RECURSIVE names(fqname,symbol) AS (
 SELECT fqname,id FROM declarations UNION
 SELECT i.module || '.' || i.name,n.symbol FROM names n CROSS JOIN imports i ON i.target=n.fqname
 WHERE i.kind='symbol' AND NOT EXISTS(SELECT 1 FROM declarations d WHERE d.fqname=i.module || '.' || i.name))
 SELECT * FROM names""",('fqname',),('symbol',)),
 S('base_edges',"""SELECT d.id AS owner,b.id AS base,r.__start AS ordinal FROM raw_bases r JOIN declarations d ON d.__path=r.__path AND d.__start=r.owner
 JOIN declarations b ON b.kind='class' AND b.id=coalesce(
 (SELECT c.id FROM imports i JOIN declarations c ON c.fqname=i.target AND c.kind='class'
 WHERE i.kind='symbol' AND i.module=d.module AND i.name=trim(rx_replace(rx_replace(r.name,'(?s)\\[.*$',''),'^.*\\.','')) LIMIT 1),
 (SELECT c.id FROM declarations c WHERE c.module=d.module AND c.kind='class' AND c.name=trim(rx_replace(rx_replace(r.name,'(?s)\\[.*$',''),'^.*\\.','')) LIMIT 1),
 (SELECT min(c.id) FROM declarations c WHERE c.kind='class' AND c.name=trim(rx_replace(rx_replace(r.name,'(?s)\\[.*$',''),'^.*\\.','')) HAVING count(*)=1))""",('owner',),('base',)),
 S('hierarchy',"""WITH RECURSIVE walk(owner,base,depth,route,seen) AS (
 SELECT id,id,0,'',',' || id || ',' FROM declarations WHERE kind='class' UNION ALL
 SELECT w.owner,e.base,w.depth+1,w.route || printf('%012d.',e.ordinal),w.seen || e.base || ',' FROM walk w JOIN base_edges e ON e.owner=w.base WHERE instr(w.seen,',' || e.base || ',')=0)
 SELECT owner,base,depth,route FROM (SELECT *,row_number() OVER (PARTITION BY owner,base ORDER BY depth,route) AS selected FROM walk) WHERE selected=1""",('owner',),('base',)),
 S('public_methods',f"""SELECT owner,name,symbol FROM (
 SELECT h.owner,d.name,d.id AS symbol,row_number() OVER (PARTITION BY h.owner,d.name ORDER BY h.depth,h.route) AS selected
 FROM hierarchy h JOIN declarations c ON c.id=h.base JOIN declarations d ON d.module=c.module AND d.class_name=c.name
 WHERE d.kind='method' AND substr(d.name,1,1)<>'_' AND d.name NOT IN {literals(graph.PUBLIC_METHOD_SKIP)}
 AND NOT EXISTS(SELECT 1 FROM decorators x WHERE x.__path=d.__path AND x.owner=d.__start AND rx_matches(x.name,'property$|\\.setter$|\\.deleter$'))) WHERE selected=1""",('owner',),('symbol',)),
 S('all_names','SELECT * FROM raw_all_names',('__path','owner','name')),
 S('explicit_exports',"SELECT *,row_number() OVER (PARTITION BY __path ORDER BY __start) AS selected FROM raw_all",('__path',)),
 S('export_names',"""SELECT path,module,name,target FROM imports WHERE top IS NOT NULL AND kind='symbol' AND rx_matches(path,'^data_layer/[^/]+\\.py$')
 UNION ALL SELECT __path,module,name,fqname FROM declarations WHERE class_name IS NULL AND rx_matches(__path,'^data_layer/[^/]+\\.py$')""",('module','name')),
 S('exported_roots',"""SELECT DISTINCT e.path,e.module AS export_module,e.name AS export_name,n.symbol FROM export_names e JOIN resolved_names n ON n.fqname=e.target
 LEFT JOIN explicit_exports a ON a.__path=e.path AND a.selected=1
 WHERE substr(e.name,1,1)<>'_' AND (a.__path IS NULL OR (a.array IS NOT NULL AND EXISTS(SELECT 1 FROM all_names v WHERE v.__path=e.path AND v.owner=a.__start AND v.name=e.name)))""",('symbol',)),
 ]
 return stages


def public_api_clause(module, graph):
 inputs=symbol_inputs()
 outer=A(K('function_definition'),N(I(K('function_definition'))),P('OWNER'),
     I(A(K('decorated_definition'),H(A(K('decorator'),{'regex':r'^@\s*(?:\w+\.)*direct_legacy[^.\s(]*'}))),'neighbor'))
 call_name=A(P('CALLEE'),{'regex':'^'+regex(graph.EXECUTION_NAMES)+'$|^(?:execute_grammar|grammar_fetch)'},{'regex':r'_lazy$|^lazy_'})
 inputs['raw_execution_calls']=E(A(K('call'),F('function',O(A(K('identifier'),call_name),A(K('attribute'),F('attribute',call_name)))),
     I(A(K('block'),I(outer,'neighbor')))),{'owner':'capture.OWNER.startByte','name':'capture.CALLEE'})
 stages=symbol_stages(graph)
 # Owner projections are nullable byte offsets. Preserve numeric affinity for index seeks.
 for stage in stages:
  if stage['name'] in ('decorators','all_names'):
   raw={'decorators':'raw_decorators','all_names':'raw_all_names'}[stage['name']]
   stage['query']=f'SELECT __path,__start,__end,name,CAST(owner AS INTEGER) AS owner FROM {raw}'
  if stage['name']=='base_edges':stage['query']=stage['query'].replace('FROM raw_bases r','FROM bases r')
 stages.insert(next(i for i,stage in enumerate(stages) if stage['name']=='base_edges'),
     S('bases','SELECT __path,__start,__end,name,CAST(owner AS INTEGER) AS owner FROM raw_bases',('__path','owner')))
 methods=stages.pop(next(i for i,stage in enumerate(stages) if stage['name']=='public_methods'))
 methods['query']=methods['query'].replace("WHERE d.kind='method'", "WHERE h.owner IN (SELECT symbol FROM exported_roots) AND d.kind='method'")
 stages.insert(next(i for i,stage in enumerate(stages) if stage['name']=='exported_roots')+1,methods)
 stages.append(S('execution_calls','SELECT __path,__start,__end,name,CAST(owner AS INTEGER) AS owner FROM raw_execution_calls',('__path','owner')))
 own="x.__path=d.__path AND x.owner=d.__start"
 inherited="x.__path=d.__path AND x.owner=d.class_start"
 def dec(pattern,include_class=False):
  return f"EXISTS(SELECT 1 FROM decorators x WHERE ({own}"+(f" OR {inherited}" if include_class else '')+f") AND rx_matches(x.name,{q(pattern)}))"
 legacy_sync=dec(r'(?:legacy_sync|legacy_module_sync|direct_legacy_module_sync)$')
 legacy_async=dec(r'(?:legacy_async|legacy_module_async|direct_legacy_module_async)$')
 lazy_pattern=r'pure_lazy|apply_lazy_to_public_lazy_methods|(?:^|\.)lazy$'
 lazy=f"({dec(lazy_pattern)} OR (substr(d.name,1,1)<>'_' AND rx_matches(d.name,'_lazy$') AND {dec(lazy_pattern,True)}))"
 private=dec(r'(?:^|\.)'+regex(module.PRIVATE_PUBLIC_API_DECORATORS)+'$',True)
 # A return annotation is chainable only when it names the declaring class.
 chain="(d.class_name IS NOT NULL AND (trim(d.return_annotation,' ' || char(39) || char(34))=d.class_name OR instr(trim(d.return_annotation,' ' || char(39) || char(34)),d.class_name || '[')=1))"
 true_lazy="(rx_matches(d.name,'_lazy$') OR EXISTS(SELECT 1 FROM annotation_names a WHERE a.symbol=d.id AND lower(a.name) IN ('lazyresult','lazyseq','serverplan')))"
 base="rx_replace(d.name,'_(?:lazy|async|sync)$','')"
 context=f"(rx_matches(d.__path,{q(regex(module.EXECUTION_PATH_HINTS))}) OR rx_matches(lower(coalesce(d.class_name,'')),{q(regex(module.EXECUTION_OWNER_HINTS))}) OR EXISTS(SELECT 1 FROM annotation_names a WHERE a.symbol=d.id AND lower(a.name) IN ('databaseconnection','lazyresult','lazyseq','readtransaction','serverplan','versioneddatabaseconnection','versionedreadtransaction','versionedwritetransaction','writetransaction')))"
 primitive=f"(d.kind='function' AND d.__path IN {literals(module.LAZY_PRIMITIVE_PATHS)} AND instr(d.name,'lazy_')=1 AND {true_lazy})"
 internal=f"(d.__path IN {literals(module.RUNTIME_INTERNAL_CONTEXT_PATHS)} AND d.class_name IN {literals(module.RUNTIME_INTERNAL_CONTEXT_NAMES)})"
 abstract=dec('abstractmethod$')
 execution=f"(NOT {abstract} AND NOT {private} AND NOT {primitive} AND NOT coalesce({internal},0) AND NOT coalesce({chain},0) AND (({base} IN {literals(module.EXECUTION_BASE_NAMES)} AND {context}) OR {true_lazy} OR {legacy_sync} OR {legacy_async}))"
 stages.append(S('traits',f"""SELECT d.*,{legacy_sync} AS legacy_sync,{legacy_async} AS legacy_async,{lazy} AS lazy_decorated,
 {private} AS private_api,{execution} AS execution_surface,
 coalesce((SELECT json_group_array(name) FROM (SELECT name FROM parameters p WHERE p.symbol=d.id ORDER BY p.__start)),'[]') AS params,
 {dec(r'(?:^|\.)direct_legacy[^.]*$')} AND EXISTS(SELECT 1 FROM execution_calls e WHERE e.__path=d.__path AND e.owner=d.__start
 AND (e.name IN {literals(graph.EXECUTION_NAMES)} OR instr(e.name,'execute_grammar')=1 OR instr(e.name,'grammar_fetch')=1)
 AND (rx_matches(e.name,'_lazy$') OR instr(e.name,'lazy_')=1)) AS direct_lazy_wrapper
 FROM declarations d""",('id',)))
 stages.append(S('surfaces',"""SELECT *,CASE WHEN mixed THEN name ELSE rx_replace(name,'_(?:lazy|async|sync)$','') END AS base_name,
 CASE WHEN mixed AND scope='method' THEN '__invalid_mixed_name__' WHEN rx_matches(name,'_lazy$') THEN 'lazy' WHEN rx_matches(name,'_async$') THEN 'async'
 WHEN legacy_async THEN 'async' ELSE 'sync' END AS slot FROM (
 SELECT d.id AS symbol,d.name,d.legacy_async,d.execution_surface,'function' AS scope,r.export_module AS owner,r.export_module || char(0) || r.export_name AS ordinal,
 rx_matches(lower(d.name),'lazy') AND rx_matches(lower(d.name),'async|sync') AS mixed
 FROM exported_roots r JOIN traits d ON d.id=r.symbol WHERE d.kind='function' AND substr(d.name,1,1)<>'_' AND instr(d.__path,'_internal/')<>1
 UNION ALL
 SELECT d.id,d.name,d.legacy_async,d.execution_surface,'method',c.name,d.name,
 rx_matches(lower(d.name),'lazy') AND rx_matches(lower(d.name),'async|sync')
 FROM (SELECT DISTINCT symbol FROM exported_roots) r CROSS JOIN traits c ON c.id=r.symbol AND c.kind='class'
 CROSS JOIN public_methods p ON p.owner=c.id CROSS JOIN traits d ON d.id=p.symbol
 WHERE NOT c.private_api AND NOT d.private_api AND NOT rx_matches(c.name,'Protocol$|^Async')
 AND NOT EXISTS(SELECT 1 FROM bases b WHERE b.__path=c.__path AND b.owner=c.__start AND instr(b.name,'Protocol')>0))""",('scope','owner','base_name')))
 stages.append(S('slots',"""SELECT * FROM (
 SELECT *,min(ordinal) OVER (PARTITION BY scope,owner,base_name,slot) AS first_ordinal,
 row_number() OVER (PARTITION BY scope,owner,base_name,slot ORDER BY ordinal DESC) AS selected
 FROM surfaces WHERE NOT (scope='function' AND mixed)) WHERE selected=1""",('scope','owner','base_name'),('symbol',)))
 stages.append(S('family_groups',"""SELECT scope,owner,base_name,
 (SELECT symbol FROM slots x WHERE x.scope=s.scope AND x.owner=s.owner AND x.base_name=s.base_name ORDER BY x.first_ordinal LIMIT 1) AS exemplar,
 max(CASE WHEN slot='sync' THEN symbol END) AS sync_id,max(CASE WHEN slot='async' THEN symbol END) AS async_id,max(CASE WHEN slot='lazy' THEN symbol END) AS lazy_id,
 max(mixed) AS mixed,max(execution_surface) AS execution_surface,
 (SELECT json_group_array(json_array(slot,symbol)) FROM (SELECT slot,symbol FROM slots x WHERE x.scope=s.scope AND x.owner=s.owner AND x.base_name=s.base_name ORDER BY slot)) AS implementations
 FROM slots s GROUP BY scope,owner,base_name""",('exemplar',)))
 exempt=f"""((g.base_name IN {literals(module.NON_TRIPLE_TRANSACTION_FACTORY_BASE_NAMES)} AND d.__path IN ('l2050_transaction/transaction.py','l7200_versioned_transactions/versioned_transactions.py') AND g.owner IN ('ReadTransaction','WriteTransaction','LazyVersionedReadTransaction','LazyVersionedWriteTransaction'))
 OR (g.owner='CacheTransaction' AND g.base_name IN ('commit','rollback')) OR (g.owner='BaseMigration' AND g.base_name='rollback')
 OR (g.base_name='execute_ddl' AND (g.owner IN ('SchemaTransaction','AsyncSchemaTransaction') OR d.__path='l2000_connection/schema_transaction.py'))
 OR (rx_matches(g.owner,'Connection$') AND g.base_name='table_exists')
 OR (g.sync_id IS NULL AND g.async_id IS NULL AND g.lazy_id IS NOT NULL AND (instr(lower(g.base_name),'bootstrap')>0 OR instr(lower(g.owner),'bootstrap')>0 OR g.base_name IN {literals(module.LAZY_ONLY_BOOTSTRAP_BASE_NAMES)}))
 OR g.owner IN ('LazyVersionedReadTransaction','LazyVersionedWriteTransaction','SyncSessionHandle','AsyncSessionHandle','SyncVersionHandle','AsyncVersionHandle')
 OR rx_matches(g.owner,'^(?:Sync|Async|Lazy).*(?:Transaction|SchemaTransaction|SessionHandle|VersionHandle)$'))"""
 eligible=f"SELECT g.* FROM family_groups g JOIN traits d ON d.id=g.exemplar WHERE g.mixed OR (NOT {exempt} AND coalesce(g.execution_surface,0))"
 all_scope=f"SELECT g.* FROM family_groups g JOIN traits d ON d.id=g.exemplar WHERE g.mixed OR NOT {exempt}"
 stages.append({'name':'eligible_groups','query':{'$option':'executionOnly','cases':{'true':eligible,'false':all_scope}}})
 stages.append(S('families',"""SELECT *,json_array(scope,owner,base_name,implementations) AS family FROM (
 SELECT *,row_number() OVER (PARTITION BY scope,base_name,implementations ORDER BY owner) AS selected FROM eligible_groups) WHERE mixed OR selected=1""",('family',)))
 stages.append(S('triple_violations',"""SELECT 'invalid_mixed_lazy_execution_name' AS issue,symbol,json_array(scope,owner,base_name,ordinal) AS family FROM surfaces WHERE scope='function' AND mixed
 UNION ALL SELECT 'invalid_mixed_lazy_execution_name',exemplar,family FROM families WHERE mixed
 UNION ALL SELECT 'missing_sync',exemplar,family FROM families WHERE NOT mixed AND sync_id IS NULL AND async_id IS NOT NULL
 UNION ALL SELECT 'missing_async',exemplar,family FROM families WHERE NOT mixed AND async_id IS NULL AND sync_id IS NOT NULL
 UNION ALL SELECT 'missing_lazy',exemplar,family FROM families WHERE NOT mixed AND lazy_id IS NULL
 UNION ALL SELECT 'missing_legacy_sync',f.sync_id,f.family FROM families f JOIN traits s ON s.id=f.sync_id JOIN traits l ON l.id=f.lazy_id WHERE NOT f.mixed AND s.params=l.params AND NOT s.legacy_sync
 UNION ALL SELECT 'missing_legacy_async',f.async_id,f.family FROM families f JOIN traits s ON s.id=f.async_id JOIN traits l ON l.id=f.lazy_id WHERE NOT f.mixed AND s.params=l.params AND NOT s.legacy_async
 UNION ALL SELECT 'missing_lazy_decorator',f.lazy_id,f.family FROM families f JOIN traits s ON s.id=f.lazy_id WHERE NOT f.mixed AND NOT s.lazy_decorated
 UNION ALL SELECT 'direct_wrapper_bypasses_lazy_owner',symbol,family FROM (
 SELECT f.family,s.symbol,row_number() OVER (PARTITION BY f.family ORDER BY d.__path,d.__start) AS selected
 FROM families f JOIN slots s ON s.scope=f.scope AND s.owner=f.owner AND s.base_name=f.base_name JOIN traits d ON d.id=s.symbol
 WHERE NOT f.mixed AND f.lazy_id IS NOT NULL AND s.slot IN ('sync','async') AND d.direct_lazy_wrapper) WHERE selected=1"""))
 return {'id':'exported-families','relational':{'inputs':inputs,'stages':stages,
   'query':'SELECT d.__path AS path,d.__start AS start_byte,d.__end AS end_byte,v.issue,v.family FROM triple_violations v JOIN traits d ON d.id=v.symbol',
   'key':['family','issue'],'code':'issue'},'violation':{'code':'public_triple','message':'Public execution family coverage','anchor':'witness.span'}}


def dead_code_clause(module, graph, reference_contract):
 inputs=symbol_inputs()
 fn=A(K('function_definition'),N(I(K('function_definition'))),P('OWNER'))
 nested_class=A(K('class_definition'),I(K('class_definition')),N(I(K('function_definition'))))
 # Module initialization reads class statements, method defaults and decorators.
 # Function bodies are separate reference-graph facts.
 name=A(K('identifier'),P('WORD'),
   N(I(O(K('import_statement'),K('import_from_statement')))),
   *[N({'inside':{**K(kind),'field':field,'stopBy':'neighbor'}}) for kind,field in (
       ('attribute','attribute'),('keyword_argument','name'),('function_definition','name'),('class_definition','name'))],
   N({'binds':{'form':['parameter']}}),
   O(N(I(A(K('block'),I(K('function_definition'),'neighbor')))),I(nested_class),I(A(N(O(K('class_definition'),K('function_definition'),K('decorated_definition'))),I(K('module'),'neighbor')))),
   optional(I(A(K('decorated_definition'),H(A(K('function_definition'),P('DEC_FN')))))),
   optional(I(A(K('decorated_definition'),H(A(K('class_definition'),P('DEC_CLASS')))))),
   optional(I(A(K('function_definition'),P('FUNCTION')),K('function_definition'))),
   optional(I(A(K('class_definition'),P('CLASS')))),
   optional(I(A(K('block'),P('CLASS_BODY'),I(K('class_definition'),'neighbor')))),
   optional({'inside':{**A(O(K('default_parameter'),K('typed_default_parameter')),P('DEFAULT')),'field':'value','stopBy':'end'}}),
   optional(I(A(nested_class,P('NESTED')))),
   optional(I(A(P('CONTROL'),N(O(K('class_definition'),K('function_definition'),K('decorated_definition'))),I(K('module'),'neighbor')))))
 inputs['raw_module_names']=E(name,{'name':'capture.WORD','decorated_function':'capture.DEC_FN.startByte','decorated_class':'capture.DEC_CLASS.startByte',
   'function':'capture.FUNCTION.startByte','class':'capture.CLASS.startByte','class_body':'capture.CLASS_BODY.startByte','default_value':'capture.DEFAULT.startByte','nested_class':'capture.NESTED.startByte','control':'capture.CONTROL.startByte'})
 inputs['raw_modules']=E(K('module'),{'module':'file.module'})
 dynamic=module._LITERAL_DYNAMIC_IMPORT_ATTR_RE.pattern
 inputs['raw_dynamic_imports']=E(A(K('module'),{'regex':'(?P<ENTRY>'+dynamic+')'}),{'entry':'capture.ENTRY'},each='ENTRY')
 stages=symbol_stages(graph)
 stages.append(S('bases','SELECT * FROM raw_bases',('__path','owner')))
 stages.append(S('function_headers','SELECT * FROM raw_functions',('__path','__start')))
 stages.append(S('dead_traits',"""SELECT d.*,rx_replace(d.name,'_(?:async|lazy)$','') AS base_name,CASE WHEN class_name IS NULL THEN NULL ELSE module || '::' || class_name END AS class_id,
 CASE WHEN kind='class' THEN EXISTS(SELECT 1 FROM bases b WHERE b.__path=d.__path AND b.owner=d.__start AND instr(b.name,'ABC')>0)
 ELSE EXISTS(SELECT 1 FROM decorators x WHERE x.__path=d.__path AND x.owner=d.__start AND rx_matches(x.name,'abstractmethod$')) END AS abstract,
 EXISTS(SELECT 1 FROM decorators x WHERE x.__path=d.__path AND x.owner=d.__start AND rx_matches(x.name,'property$|\\.setter$|\\.deleter$')) AS property,
 rx_matches(name,'^__.*__$') OR name IN ('main','pytest_configure','pytest_collection_modifyitems') OR
 EXISTS(SELECT 1 FROM decorators x WHERE x.__path=d.__path AND x.owner=d.__start AND rx_matches(x.name,'validator$|overload$')) AS implicit,
 rx_matches(name,'Protocol$') OR EXISTS(SELECT 1 FROM bases b WHERE b.__path=d.__path AND b.owner=d.__start AND instr(b.name,'Protocol')>0) AS protocol,
 EXISTS(SELECT 1 FROM bases b WHERE b.__path=d.__path AND b.owner=d.__start AND instr(b.name,'Protocol')>0) AS structural_protocol
 FROM declarations d""",('id',),('module','class_name','name'),('class_id','name'),('kind','name')))
 stages.append(S('class_names',"""SELECT r.module,r.name,coalesce(
 (SELECT c.id FROM imports i JOIN resolved_names n ON n.fqname=i.target JOIN declarations c ON c.id=n.symbol AND c.kind='class' WHERE i.kind='symbol' AND i.module=r.module AND i.name=r.name LIMIT 1),
 (SELECT c.id FROM declarations c WHERE c.kind='class' AND c.module=r.module AND c.name=r.name LIMIT 1)) AS target
 FROM (SELECT DISTINCT d.module,a.name FROM annotation_names a JOIN declarations d ON d.id=a.symbol) r""",('module','name')))
 stages.append(S('module_names',f"""SELECT DISTINCT {MOD} AS module,n.name FROM raw_module_names n
 WHERE n.control IS NOT NULL OR n.nested_class IS NOT NULL OR (
 (n.decorated_function IS NULL OR EXISTS(SELECT 1 FROM function_headers f WHERE f.__path=n.__path AND f.__start=n.decorated_function AND f.class_name IS NOT NULL))
 AND (n.decorated_class IS NULL OR n.class_body IS NOT NULL)
 AND (n.class IS NULL OR n.class_body IS NOT NULL)
 AND (n.function IS NULL OR (n.default_value IS NOT NULL AND EXISTS(SELECT 1 FROM function_headers f WHERE f.__path=n.__path AND f.__start=n.function AND f.class_name IS NOT NULL))))""",('module','name')))
 stages.append(S('imported_modules',"""WITH RECURSIVE modules(module) AS (
 SELECT CASE WHEN kind='module' THEN target ELSE rx_replace(target,'\\.?[^.]+$','') END FROM imports
 UNION SELECT rx_replace(module,'\\.?[^.]+$','') FROM modules WHERE instr(module,'.')>0)
 SELECT DISTINCT module FROM modules WHERE module<>''""",('module',)))
 roots="SELECT symbol FROM exported_roots UNION SELECT p.symbol FROM public_methods p JOIN exported_roots r ON r.symbol=p.owner UNION SELECT '@module:' || module FROM imported_modules"
 dynamic_roots=f" UNION SELECT n.symbol FROM raw_dynamic_imports d JOIN resolved_names n ON n.fqname=rx_extract(d.entry,{q(dynamic)},1) || '.' || rx_extract(d.entry,{q(dynamic)},2)"
 stages.append({'name':'dead_roots','query':{'$option':'oracleAnalyzer','cases':{'true':roots,'false':roots+dynamic_roots}},'indexes':[['symbol']]})
 # A module vertex lets a reached symbol initialize its module exactly once.
 stages.append(S('reach_edges',"""SELECT s.id AS source,t.id AS target FROM reference_edges e
 JOIN declarations s ON s.__path=e.__path AND s.__start=e.__start JOIN declarations t ON t.__path=e.target_path AND t.__start=e.target_start
 UNION SELECT owner,symbol FROM public_methods
 UNION SELECT h.owner,m.id FROM hierarchy h JOIN declarations c ON c.id=h.base JOIN declarations m ON m.module=c.module AND m.class_name=c.name
 WHERE m.name IN ('__new__','__init__','__init_subclass__','__post_init__','__set_name__','__class_getitem__')
 UNION SELECT owner,base FROM base_edges UNION SELECT base,owner FROM base_edges
 UNION SELECT d.id,s.id FROM dead_traits d CROSS JOIN declarations s ON s.module=d.module
 AND s.name IN (d.base_name,d.base_name || '_async',d.base_name || '_lazy') AND s.id<>d.id WHERE d.kind='function'
 UNION SELECT d.id,s.id FROM dead_traits d CROSS JOIN declarations s ON s.id IN
 (d.class_id || '.' || d.base_name,d.class_id || '.' || d.base_name || '_async',d.class_id || '.' || d.base_name || '_lazy') AND s.id<>d.id WHERE d.kind='method'
 UNION SELECT a.symbol,c.target FROM annotation_names a JOIN declarations d ON d.id=a.symbol JOIN class_names c ON c.module=d.module AND c.name=a.name WHERE c.target IS NOT NULL
 UNION SELECT d.id,'@module:' || d.module FROM declarations d
 UNION SELECT '@module:' || n.module,d.id FROM module_names n JOIN declarations d ON d.module=n.module AND d.name=n.name
 UNION SELECT '@module:' || n.module,d.id FROM module_names n JOIN imports i ON i.module=n.module AND i.name=n.name AND i.kind='symbol' JOIN declarations d ON d.fqname=i.target
 WHERE NOT EXISTS(SELECT 1 FROM declarations local WHERE local.module=n.module AND local.name=n.name)
 UNION SELECT m.id,other.id FROM dead_traits m JOIN dead_traits c ON c.id=m.class_id AND c.structural_protocol
 JOIN dead_traits other ON other.kind='method' AND other.name=m.name AND other.class_id<>m.class_id""",('source',),('target',)))
 # Dispatch states retain the original method while traversing subclasses and
 # co-bases. UNION visits each state once, including cyclic class graphs.
 stages.append(S('reachable',"""WITH RECURSIVE flow(node,dispatch) AS (
 SELECT symbol,NULL FROM dead_roots
 UNION SELECT e.target,NULL FROM flow f JOIN reach_edges e ON e.source=f.node WHERE f.dispatch IS NULL
 UNION SELECT b.owner,f.node FROM flow f JOIN dead_traits m ON m.id=f.node JOIN base_edges b ON b.base=m.class_id WHERE f.dispatch IS NULL AND m.kind='method'
 UNION SELECT b.owner,f.dispatch FROM flow f JOIN base_edges b ON b.base=f.node WHERE f.dispatch IS NOT NULL
 UNION SELECT b.base,f.dispatch FROM flow f JOIN base_edges b ON b.owner=f.node JOIN dead_traits original ON original.id=f.dispatch WHERE f.dispatch IS NOT NULL AND b.base<>original.class_id
 UNION SELECT m.id,NULL FROM flow f JOIN dead_traits original ON original.id=f.dispatch JOIN declarations c ON c.id=f.node JOIN dead_traits m ON m.module=c.module AND m.class_name=c.name AND m.name=original.name WHERE f.dispatch IS NOT NULL)
 SELECT DISTINCT node AS symbol FROM flow WHERE dispatch IS NULL""",('symbol',)))
 stages.append(S('annotation_reachable',"""WITH RECURSIVE unique_classes(name,target) AS (SELECT name,min(id) FROM declarations WHERE kind='class' GROUP BY name HAVING count(*)=1),
 annotated(symbol) AS (SELECT symbol FROM reachable UNION SELECT c.target FROM annotated r JOIN annotation_names a ON a.symbol=r.symbol JOIN unique_classes c ON c.name=a.name)
 SELECT symbol FROM annotated""",('symbol',)))
 stages.append(S('high_confidence',"""SELECT d.* FROM dead_traits d
 WHERE NOT EXISTS(SELECT 1 FROM annotation_reachable r WHERE r.symbol=d.id)
 AND NOT rx_matches(d.module,'^(?:api|data_layer)\\.') AND NOT d.implicit
 AND NOT EXISTS(SELECT 1 FROM imports i WHERE i.kind='symbol' AND i.target=d.fqname)
 AND NOT (d.kind='class' AND d.protocol)
 AND NOT (d.kind<>'class' AND (d.abstract OR d.property OR instr(d.name,'_lower_')=1 OR instr(lower(d.name),'register')>0
   OR rx_matches(d.name,'^_create_.*_(?:connection|validator|factory)$')
   OR EXISTS(SELECT 1 FROM dead_traits c WHERE c.id=d.class_id AND c.protocol)
   OR EXISTS(SELECT 1 FROM hierarchy h JOIN dead_traits c ON c.id=h.base JOIN dead_traits m ON m.module=c.module AND m.class_name=c.name AND m.name=d.name AND m.abstract WHERE h.owner=d.class_id AND h.depth>0)))
 AND NOT EXISTS(SELECT 1 FROM hierarchy h JOIN dead_traits b ON b.id=h.base JOIN annotation_reachable r ON r.symbol=b.id
   WHERE h.owner=CASE WHEN d.kind='class' THEN d.id ELSE d.class_id END AND h.depth>0 AND
   (b.abstract OR (instr(b.name,'ABC')>0 AND EXISTS(SELECT 1 FROM base_edges e WHERE e.owner=h.owner AND e.base=b.id))))""",('id',)))
 intent="rx_matches(d.name,'Intent$') OR EXISTS(SELECT 1 FROM bases b WHERE b.__path=d.__path AND b.owner=d.__start AND instr(b.name,'ServerPlanStep')>0)"
 grammar="NOT d.protocol AND (instr(d.__path,'/l2100_sql_grammar/')>0 OR rx_matches(d.name,'Grammar$|^Sql') OR EXISTS(SELECT 1 FROM bases b WHERE b.__path=d.__path AND b.owner=d.__start AND rx_matches(b.name,'Sql|Grammar')))"
 categories=f"""SELECT d.*, 'high_confidence_functions' AS category FROM high_confidence d WHERE kind<>'class'
 UNION ALL SELECT d.*,'high_confidence_intents' FROM high_confidence d WHERE kind='class' AND ({intent})
 UNION ALL SELECT d.*,'high_confidence_grammar_classes' FROM high_confidence d WHERE kind='class' AND ({grammar})
 UNION ALL SELECT d.*,'high_confidence_other_classes' FROM high_confidence d WHERE kind='class' AND NOT ({intent}) AND NOT ({grammar})"""
 final="SELECT __path AS path,__start AS start_byte,__end AS end_byte,id AS symbol,category FROM ("+categories+")"
 return {'id':'unreachable-internal-symbols','relational':{'inputs':inputs,'references':{'language':'python',**reference_contract},'stages':stages,
   'query':{'$option':'oracleAnalyzer','cases':{'true':final,'false':final+" WHERE instr(__path,'impl/browser/')<>1"}},'key':['symbol','category'],'code':'category'},
   'violation':{'code':'internal_dead_code','message':'Unreachable internal symbol','anchor':'witness.span'}}
