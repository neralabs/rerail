# SPDX-License-Identifier: Apache-2.0
"""Relational step contracts, local bindings, and helper return summaries."""
from .common import *
from .ast_relations import relational,python_stages,ast_stages
from .step_regions import family,regions

def contract_clause(m):
 stages=python_stages()+[
 S('step_classes',"SELECT c.* FROM classes c WHERE EXISTS(SELECT 1 FROM nodes args JOIN nodes b ON b.__path=args.__path AND b.parent=args.id WHERE args.__path=c.__path AND args.parent=c.id AND args.kind='argument_list' AND rx_matches(b.text,'^(?:[A-Za-z_]\\w*\\.)*ServerPlanStep(?:\\[.*\\])?$'))",('__path','id')),
 S('step_methods',"SELECT m.* FROM methods m JOIN step_classes c ON c.__path=m.__path AND c.id=m.cls",('__path','cls','name'),('__path','id')),
 S('first_methods',"SELECT * FROM (SELECT *,row_number() OVER(PARTITION BY __path,cls,name ORDER BY __start) AS selected FROM step_methods) WHERE selected=1",('__path','cls','name'),('__path','id')),
 S('method_descendants',"SELECT m.__path,m.id AS method,m.cls,m.name,d.node FROM step_methods m JOIN descendants d ON d.__path=m.__path AND d.root=m.id",('__path','method','node'),('__path','node','name')),
 S('method_returns',"SELECT m.__path,m.id AS method,m.cls,m.name,r.id,r.value,r.expr_kind,r.expr_text FROM first_methods m JOIN returns r ON r.__path=m.__path AND r.fn=m.id",('__path','cls','name'),('__path','method')),
 S('empty_tables',"""SELECT DISTINCT r.__path,r.method,r.cls,r.name FROM method_returns r JOIN calls c ON c.__path=r.__path AND c.id=r.value
 WHERE c.full_callee='frozenset' AND NOT EXISTS(SELECT 1 FROM keywords k WHERE k.__path=c.__path AND k.call=c.id)
 AND (NOT EXISTS(SELECT 1 FROM arguments a WHERE a.__path=c.__path AND a.call=c.id) OR
 ((SELECT count(*) FROM arguments a WHERE a.__path=c.__path AND a.call=c.id)=1 AND EXISTS(SELECT 1 FROM arguments a WHERE a.__path=c.__path AND a.call=c.id AND a.kind IN ('list','tuple','set') AND NOT EXISTS(SELECT 1 FROM nodes v WHERE v.__path=a.__path AND v.parent=a.id))))""",('__path','cls','name')),
 S('roles',"SELECT DISTINCT __path,cls,CASE WHEN expr_kind='attribute' THEN rx_extract(expr_text,'^StepExecutionRole\\.(VIRTUAL|SQL|APPLY|CONTROL)$',1) ELSE upper(trim(expr_text,char(34)||char(39))) END AS role FROM method_returns WHERE name='execution_role'",('__path','cls','role')),
 S('levels',"SELECT __path,cls,CAST(text AS INTEGER) AS level FROM (SELECT m.__path,m.cls,n.text,row_number() OVER(PARTITION BY m.__path,m.cls ORDER BY n.__start) AS selected FROM method_descendants m JOIN nodes n ON n.__path=m.__path AND n.id=m.node WHERE m.name='level' AND n.kind='integer' AND EXISTS(SELECT 1 FROM returns r WHERE r.__path=n.__path AND r.value=n.id)) WHERE selected=1",('__path','cls')),
 S('nonempty_keywords',"SELECT * FROM keywords k WHERE NOT (k.kind='none' OR (k.kind IN ('string','concatenated_string') AND trim(k.text,char(34)||char(39))='') OR (k.kind IN ('list','tuple','set') AND NOT EXISTS(SELECT 1 FROM nodes n WHERE n.__path=k.__path AND n.parent=k.value)))",('__path','call','name')),
 S('compiled',"""SELECT c.*,NOT EXISTS(SELECT 1 FROM keywords k WHERE k.__path=c.__path AND k.call=c.id AND k.name='grammar' AND k.kind!='none') AS grammarless,
 EXISTS(SELECT 1 FROM nonempty_keywords k WHERE k.__path=c.__path AND k.call=c.id AND k.name IN ('runtime_ref_ids','source_refs')) AS refs,
 EXISTS(SELECT 1 FROM nonempty_keywords k WHERE k.__path=c.__path AND k.call=c.id AND k.name IN ('runtime_ref_ids','source_refs','auxiliary_ctes')) OR EXISTS(SELECT 1 FROM keywords k WHERE k.__path=c.__path AND k.call=c.id AND k.name='grammar' AND k.kind!='none') AS sql_surface
 FROM calls c WHERE c.full_callee='CompiledStep'""",('__path','id')),
 S('compile_surfaces',"SELECT DISTINCT m.__path,m.cls,m.id AS method FROM first_methods m JOIN descendants d ON d.__path=m.__path AND d.root=m.id JOIN compiled c ON c.__path=d.__path AND c.id=d.node AND c.sql_surface WHERE m.name='compile'",('__path','cls')),
 S('zero_io',"""SELECT c.__path,c.id AS cls FROM step_classes c JOIN levels l ON l.__path=c.__path AND l.cls=c.id AND l.level=0
 JOIN empty_tables r ON r.__path=c.__path AND r.cls=c.id AND r.name='tables_read' JOIN empty_tables w ON w.__path=c.__path AND w.cls=c.id AND w.name='tables_written'
 JOIN first_methods m ON m.__path=c.__path AND m.cls=c.id AND m.name='compile'
 WHERE EXISTS(SELECT 1 FROM descendants d JOIN compiled cs ON cs.__path=d.__path AND cs.id=d.node WHERE d.__path=m.__path AND d.root=m.id AND
 (NOT EXISTS(SELECT 1 FROM arguments a WHERE a.__path=cs.__path AND a.call=cs.id) AND NOT EXISTS(SELECT 1 FROM keywords k WHERE k.__path=cs.__path AND k.call=cs.id) OR EXISTS(SELECT 1 FROM keywords k WHERE k.__path=cs.__path AND k.call=cs.id AND k.name IN ('pre_op','post_op','on_commit','on_rollback') AND k.kind!='none')))""",('__path','cls')),
 S('local_bindings',"SELECT * FROM (SELECT f.__path,f.id AS owner,a.target_text AS name,a.value,row_number() OVER(PARTITION BY f.__path,f.id,a.target_text ORDER BY a.__start DESC) AS selected FROM functions f JOIN assignments a ON a.__path=f.__path AND a.__start>f.__start AND a.__end<=f.__end WHERE a.target_kind='identifier' AND a.value IS NOT NULL) WHERE selected=1",('__path','owner','name')),
 S('expression_edges',"""SELECT p.__path,c.id AS src,p.id AS dst FROM nodes p JOIN nodes c ON c.__path=p.__path AND c.parent=p.id WHERE p.kind IN ('tuple','list','set','expression_list','parenthesized_expression','keyword_argument','argument_list')
 UNION SELECT p.__path,c.id,p.id FROM nodes p JOIN nodes c ON c.__path=p.__path AND c.parent=p.id AND c.ordinal=1 WHERE p.kind='pair'
 UNION SELECT p.__path,c.id,p.id FROM nodes p JOIN nodes c ON c.__path=p.__path AND c.parent=p.id WHERE p.kind='dictionary'
 UNION SELECT c.__path,a.id,c.id FROM calls c JOIN arguments a ON a.__path=c.__path AND a.call=c.id
 UNION SELECT c.__path,k.value,c.id FROM calls c JOIN keywords k ON k.__path=c.__path AND k.call=c.id
 UNION SELECT n.__path,b.value,n.id FROM nodes n JOIN ownership o ON o.__path=n.__path AND o.id=n.id JOIN local_bindings b ON b.__path=n.__path AND b.owner=o.fn AND b.name=n.text WHERE n.kind='identifier'
 UNION SELECT c.__path,r.value,c.id FROM calls c JOIN functions f ON f.__path=c.__path AND f.name=c.full_callee JOIN returns r ON r.__path=f.__path AND r.fn=f.id
 """,('__path','src'),('__path','dst')),
 S('runtime_consumers',"""WITH RECURSIVE consumers(__path,id) AS (
 SELECT c.__path,c.id FROM calls c JOIN nonempty_keywords k ON k.__path=c.__path AND k.call=c.id AND k.name IN ('runtime_ref_ids','source_refs')
 UNION SELECT x.__path,e.dst FROM consumers x JOIN expression_edges e ON e.__path=x.__path AND e.src=x.id) SELECT * FROM consumers""",('__path','id')),
 S('refless_consumers',"""WITH RECURSIVE consumers(__path,id) AS (
 SELECT c.__path,c.id FROM calls c WHERE c.callee='runtime_refs_post_op' AND NOT EXISTS(SELECT 1 FROM nonempty_keywords k WHERE k.__path=c.__path AND k.call=c.id AND k.name IN ('runtime_ref_ids','source_refs'))
 UNION SELECT x.__path,e.dst FROM consumers x JOIN expression_edges e ON e.__path=x.__path AND e.src=x.id JOIN nodes n ON n.__path=e.__path AND n.id=e.dst
 WHERE NOT EXISTS(SELECT 1 FROM calls c WHERE c.__path=n.__path AND c.id=n.id AND c.callee='runtime_refs_post_op')
 UNION SELECT n.__path,n.id FROM consumers x JOIN returns r ON r.__path=x.__path AND r.value=x.id JOIN functions f ON f.__path=r.__path AND f.id=r.fn JOIN nodes n ON n.__path=f.__path AND n.kind='identifier' AND n.text=f.name)
 SELECT * FROM consumers""",('__path','id')),
 ]
 rows=[]
 def add(entry,issue,table,where,identity='n.id'):
  rows.append(f"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,{q(issue)} AS issue,{identity} AS identity,{q(entry)} AS entry FROM {table} n WHERE {where}")
 exists_method=lambda name,cls='n.cls':f"EXISTS(SELECT 1 FROM first_methods m WHERE m.__path=n.__path AND m.cls={cls} AND m.name={q(name)})"
 role=lambda name:f"EXISTS(SELECT 1 FROM roles r WHERE r.__path=n.__path AND r.cls=n.cls AND r.role={q(name)})"
 add('check_compile_must_not_raise','STEP-018','nodes',"n.kind='raise_statement' AND EXISTS(SELECT 1 FROM step_methods m JOIN levels l ON l.__path=m.__path AND l.cls=m.cls AND l.level=0 WHERE m.__path=n.__path AND m.name='compile' AND n.parent=m.body_kind||':'||m.body_start||':'||m.body_end) AND EXISTS(SELECT 1 FROM nodes e WHERE e.__path=n.__path AND e.parent=n.id AND (e.text IN ('TypeError','NotImplementedError') OR EXISTS(SELECT 1 FROM calls c WHERE c.__path=e.__path AND c.id=e.id AND c.full_callee IN ('TypeError','NotImplementedError'))))")
 add('check_zero_io_l0_execution_role_guardrail','STEP-023','step_classes',"EXISTS(SELECT 1 FROM zero_io z WHERE z.__path=n.__path AND z.cls=n.id) AND NOT "+exists_method('execution_role','n.id'))
 add('check_zero_io_l0_execution_role_enum_semantics','STEP-024','first_methods',"n.name='execution_role' AND EXISTS(SELECT 1 FROM zero_io z WHERE z.__path=n.__path AND z.cls=n.cls) AND EXISTS(SELECT 1 FROM nodes t WHERE t.__path=n.__path AND t.id=n.return_type_kind||':'||n.return_type_start||':'||n.return_type_end AND t.text IN ('str',char(34)||'str'||char(34),char(39)||'str'||char(39)))")
 add('check_zero_io_l0_execution_role_enum_semantics','STEP-025','first_methods',"n.name='execution_role' AND EXISTS(SELECT 1 FROM zero_io z WHERE z.__path=n.__path AND z.cls=n.cls) AND EXISTS(SELECT 1 FROM method_returns r WHERE r.__path=n.__path AND r.method=n.id AND r.expr_kind IN ('string','concatenated_string') AND trim(r.expr_text,char(34)||char(39)) IN ('sql','virtual','apply','control'))")
 add('check_sql_visible_steps_declare_cte_type_hints','STEP-034','first_methods',"n.name='cte_output_columns' AND NOT "+exists_method('cte_column_type_hints')+" AND NOT EXISTS(SELECT 1 FROM descendants d JOIN returns r ON r.__path=d.__path AND r.id=d.node WHERE d.__path=n.__path AND d.root=n.id AND r.expr_kind='tuple' AND NOT EXISTS(SELECT 1 FROM nodes x WHERE x.__path=r.__path AND x.parent=r.value))")
 add('check_runtime_refs_post_op_requires_runtime_refs_guardrail','STEP-034','functions',"EXISTS(SELECT 1 FROM functions f JOIN returns r ON r.__path=f.__path AND r.fn=f.id JOIN refless_consumers c ON c.__path=r.__path AND c.id=r.value WHERE f.__path=n.__path AND f.name=n.name)")
 for entry,issue,has_refs in [('check_runtime_ref_closed_post_op_consumption_guardrail','STEP-033',True),('check_helper_anchored_runtime_ref_post_op_guardrail','STEP-035',False)]:
  add(entry,issue,'nodes',"EXISTS(SELECT 1 FROM keywords k JOIN runtime_consumers c ON c.__path=k.__path AND c.id=k.value WHERE k.__path=n.__path AND k.value=n.id AND k.name='closed_post_op' AND "+('' if has_refs else 'NOT ')+"EXISTS(SELECT 1 FROM nonempty_keywords r WHERE r.__path=k.__path AND r.call=k.call AND r.name='runtime_ref_ids')) AND EXISTS(SELECT 1 FROM functions f WHERE f.__path=n.__path AND f.__start<n.__start AND f.__end>n.__end)")
 # Role contracts use the first class method, matching the source lookup.
 add('check_virtual_execute_runtime_python_shaping_guardrail','STEP-036','first_methods',"n.name='execute_runtime' AND "+role('VIRTUAL')+" AND EXISTS(SELECT 1 FROM first_methods m JOIN descendants d ON d.__path=m.__path AND d.root=m.id JOIN compiled c ON c.__path=d.__path AND c.id=d.node WHERE m.__path=n.__path AND m.cls=n.cls AND m.name='compile' AND c.grammarless)")
 add('check_virtual_execute_runtime_scalar_value_guardrail','STEP-033','first_methods',"n.name='compile' AND "+role('VIRTUAL')+' AND '+exists_method('execute_runtime')+" AND EXISTS(SELECT 1 FROM descendants d JOIN compiled c ON c.__path=d.__path AND c.id=d.node JOIN keywords k ON k.__path=c.__path AND k.call=c.id AND k.name='closed_post_op' JOIN calls scalar ON scalar.__path=k.__path AND scalar.id=k.value AND scalar.full_callee='scalar_column_post_op' WHERE d.__path=n.__path AND d.root=n.id AND c.grammarless AND NOT c.refs AND (EXISTS(SELECT 1 FROM arguments a WHERE a.__path=scalar.__path AND a.call=scalar.id AND a.position=0 AND trim(a.text,char(34)||char(39))='value') OR EXISTS(SELECT 1 FROM keywords col WHERE col.__path=scalar.__path AND col.call=scalar.id AND col.name='column_name' AND trim(col.text,char(34)||char(39))='value')))")
 add('check_step_role_contract_guardrails','STEP-046','first_methods',"n.name='compile' AND "+role('VIRTUAL')+" AND EXISTS(SELECT 1 FROM compile_surfaces s WHERE s.__path=n.__path AND s.cls=n.cls)")
 add('check_step_role_contract_guardrails','STEP-046','step_classes',"EXISTS(SELECT 1 FROM roles r WHERE r.__path=n.__path AND r.cls=n.id AND r.role='VIRTUAL') AND ("+exists_method('cte_publication_contract','n.id')+' OR '+exists_method('publication_contract','n.id')+')')
 add('check_step_role_contract_guardrails','STEP-047','step_classes',"EXISTS(SELECT 1 FROM roles r WHERE r.__path=n.__path AND r.cls=n.id AND r.role='VIRTUAL') AND ("+exists_method('execute_runtime','n.id')+' OR '+exists_method('execute_runtime_async','n.id')+" OR EXISTS(SELECT 1 FROM method_returns r WHERE r.__path=n.__path AND r.cls=n.id AND r.name='is_executor_owned_runtime_step' AND r.expr_kind='true'))")
 add('check_step_role_contract_guardrails','STEP-048','first_methods',"n.name='step_tags' AND "+role('VIRTUAL')+" AND EXISTS(SELECT 1 FROM descendants d JOIN nodes s ON s.__path=d.__path AND s.id=d.node WHERE d.__path=n.__path AND d.root=n.id AND s.kind IN ('string','concatenated_string') AND trim(s.text,char(34)||char(39)) IN "+literals(m.SQLISH_STEP_TAGS)+')')
 add('check_step_role_contract_guardrails','STEP-049','first_methods',"n.name='supports_sql_batch_transport' AND NOT "+role('SQL')+" AND EXISTS(SELECT 1 FROM method_returns r WHERE r.__path=n.__path AND r.method=n.id AND r.expr_kind='true') AND NOT "+exists_method('cte_publication_contract')+" AND NOT EXISTS(SELECT 1 FROM compile_surfaces s WHERE s.__path=n.__path AND s.cls=n.cls)")
 add('check_step_role_contract_guardrails','STEP-050','first_methods',"n.name='emits_sql_for_accounting' AND EXISTS(SELECT 1 FROM method_returns r WHERE r.__path=n.__path AND r.method=n.id AND r.expr_kind='false') AND EXISTS(SELECT 1 FROM compile_surfaces s WHERE s.__path=n.__path AND s.cls=n.cls)")
 # Add graph-backed detectors before constructing entry-point selection.
 add_graph_checks(m,stages,add)
 # Each detector keeps only its table dependencies. Group compatible detectors
 # while preserving the engine's 32-stage bound.
 all_stages=ast_stages()+stages
 by_name={stage['name']:stage for stage in all_stages}
 def dependencies(sql):
  names=set(re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',sql)) & by_name.keys()
  pending=list(names)
  while pending:
   for name in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',by_name[pending.pop()]['query']):
    if name in by_name and name not in names:names.add(name);pending.append(name)
  return names
 groups=[]
 from .step_returns import ENTRIES as RETURN_ENTRIES
 for row in rows:
  entry=row.split(' AS entry FROM ')[0].rsplit(',',1)[-1].strip("'")
  if entry in RETURN_ENTRIES:continue
  needed=dependencies(row)
  if len(needed)>32:raise ValueError(f'{entry} requires {len(needed)} stages')
  region_family=family(entry)
  group=next((g for g in groups if g['family']==region_family and len(g['names']|needed)<=32),None)
  if group is None:group={'rows':[],'names':set(),'family':region_family};groups.append(group)
  group['names']|=needed;group['rows'].append((entry,row))
 results=[]
 for i,group in enumerate(groups):
  entries=sorted({entry for entry,_ in group['rows']})
  query=' UNION ALL '.join(row for _,row in group['rows'])
  selection={'$option':'entryPoint','cases':{
   **{e:query for e in ['all','_scan_source','_scan_file']},
   **{entry:' UNION ALL '.join(row for e,row in group['rows'] if e==entry) for entry in entries}},
   'default':'SELECT * FROM ('+query+') WHERE 0'}
  result=relational('step-contracts-'+str(i+1),[],selection,key=('entry','issue','identity'))
  import copy
  result['relational']['stages']=[copy.deepcopy(stage) for stage in all_stages if stage['name'] in group['names']]
  region=regions(group['family'])
  if group['family']=='legacy':
   strict=O({'matches':'step-strict-module'},I({'matches':'step-strict-module'}))
   result['relational']['inputs']['raw_nodes']['evidence']['query']['ast']['rule']['all'].append(choice('strictModulesOnly',strict,TRUE))
  result['relational']['inputs']['raw_nodes']['evidence']['query']['ast']['rule']['all'].extend([gate([*entries,'_scan_source','_scan_file']),region,optional(I(A(region,P('ANCESTOR')),region))])
  result['relational']['inputs']['raw_nodes']['evidence']['project'].update({'ancestor_start':'capture.ANCESTOR.startByte','ancestor_end':'capture.ANCESTOR.endByte','ancestor_kind':'capture.ANCESTOR.kind'})
  result['relational']['inputs']['relaxed']=E(A(K('module'),choice('strictModulesOnly',FALSE,TRUE)),{'path':'file.path'})
  results.append(result)
 return results

def add_graph_checks(m,stages,add):
 stages.extend([
 S('resolved_attributes',"SELECT DISTINCT m.__path,m.cls,a.attr FROM step_methods m JOIN descendants d ON d.__path=m.__path AND d.root=m.id JOIN assignments b ON b.__path=d.__path AND b.id=d.node JOIN attributes a ON a.__path=b.__path AND a.id=b.target WHERE m.name!='__init__' AND a.object_text='self' AND substr(a.attr,1,1)='_'",('__path','cls','attr')),
 S('compile_aliases',"SELECT DISTINCT m.__path,m.id AS method,a.target_text AS name,p.attr FROM step_methods m JOIN assignments a ON a.__path=m.__path AND a.fn=m.id JOIN attributes p ON p.__path=a.__path AND p.id=a.value WHERE m.name='compile' AND a.target_kind='identifier' AND p.object_text='self' AND substr(p.attr,1,1)='_'",('__path','method','name')),
 S('callback_links',"""SELECT m.__path,m.id AS method,m.cls,c.id AS call,k.keyword,k.name AS kind,f.id AS callback,f.name AS callback_name FROM step_methods m JOIN compiled c ON c.__path=m.__path AND c.__start>m.__start AND c.__end<=m.__end
 JOIN ownership o ON o.__path=c.__path AND o.id=c.id AND o.fn=m.id JOIN keywords k ON k.__path=c.__path AND k.call=c.id AND k.name IN ('post_op','post_op_with_refs','on_commit','on_rollback') AND k.kind='identifier'
 JOIN functions f ON f.__path=k.__path AND f.name=k.text AND f.__start>m.__start AND f.__end<=m.__end
 WHERE m.name='compile' AND NOT EXISTS(SELECT 1 FROM functions newer WHERE newer.__path=f.__path AND newer.name=f.name AND newer.__start>f.__start AND newer.__end<=m.__end)""",('__path','callback')),
 S('authority_parameters',"SELECT p.* FROM parameters p JOIN descendants d ON d.__path=p.__path AND d.root=p.annotation JOIN nodes n ON n.__path=d.__path AND n.id=d.node WHERE n.kind='identifier' AND n.text IN " + literals(m.AUTHORITY_TYPE_NAMES),('__path','fn','name')),
 S('authority_returns',"SELECT DISTINCT f.__path,f.name FROM functions f JOIN descendants d ON d.__path=f.__path AND d.root=f.return_type_kind||':'||f.return_type_start||':'||f.return_type_end JOIN nodes n ON n.__path=d.__path AND n.id=d.node WHERE n.kind='identifier' AND n.text IN " + literals(m.AUTHORITY_TYPE_NAMES),('__path','name')),
 S('callback_carriers',"SELECT DISTINCT l.__path,l.keyword,p.name FROM callback_links l JOIN parameters p ON p.__path=l.__path AND p.fn=l.callback WHERE (l.kind IN ('post_op','post_op_with_refs') AND p.position=1) OR EXISTS(SELECT 1 FROM descendants d JOIN nodes n ON n.__path=d.__path AND n.id=d.node WHERE d.__path=p.__path AND d.root=p.annotation AND n.kind='identifier' AND n.text='Hydrator')",('__path','keyword','name')),
 S('bad_callback_nodes',"""SELECT l.__path,l.keyword,n.id,n.__start,n.__end FROM callback_links l JOIN nodes n ON n.__path=l.__path JOIN ownership o ON o.__path=n.__path AND o.id=n.id AND o.fn=l.callback
 WHERE EXISTS(SELECT 1 FROM attributes a JOIN callback_carriers p ON p.__path=a.__path AND p.keyword=l.keyword AND p.name=a.object_text WHERE a.__path=n.__path AND a.id=n.id AND a.attr IN """+literals(m.CALLBACK_RUNTIME_AUTHORITY_ATTRS)+""") OR EXISTS(
 SELECT 1 FROM calls c WHERE c.__path=n.__path AND c.id=n.id AND (
 c.callee_kind='identifier' AND (c.callee IN """+literals(m.CALLBACK_RUNTIME_AUTHORITY_HELPERS)+""" OR EXISTS(SELECT 1 FROM authority_returns f WHERE f.__path=c.__path AND f.name=c.callee))
 OR EXISTS(SELECT 1 FROM arguments a JOIN authority_parameters p ON p.__path=a.__path AND p.fn=l.callback AND p.name=a.text WHERE a.__path=c.__path AND a.call=c.id AND a.kind='identifier')
 OR c.full_callee='getattr' AND EXISTS(SELECT 1 FROM arguments a JOIN callback_carriers p ON p.__path=a.__path AND p.keyword=l.keyword AND p.name=a.text WHERE a.__path=c.__path AND a.call=c.id AND a.position=0 AND a.kind='identifier') AND EXISTS(SELECT 1 FROM arguments a WHERE a.__path=c.__path AND a.call=c.id AND a.position=1 AND trim(a.text,char(34)||char(39)) IN """+literals(m.CALLBACK_RUNTIME_AUTHORITY_ATTRS)+""")))""",('__path','id')),
 ])
 add('check_compile_no_data_branching','STEP-016','nodes',"n.kind IN ('if_statement','elif_clause') AND EXISTS(SELECT 1 FROM ownership o JOIN step_methods m ON m.__path=o.__path AND m.id=o.fn AND m.name='compile' JOIN descendants d ON d.__path=n.__path AND d.root=n.condition_kind||':'||n.condition_start||':'||n.condition_end JOIN nodes x ON x.__path=d.__path AND x.id=d.node WHERE o.__path=n.__path AND o.id=n.id AND (EXISTS(SELECT 1 FROM attributes a JOIN resolved_attributes r ON r.__path=a.__path AND r.cls=m.cls AND r.attr=a.attr WHERE a.__path=x.__path AND a.id=x.id AND a.object_text='self') OR x.kind='identifier' AND EXISTS(SELECT 1 FROM compile_aliases a JOIN resolved_attributes r ON r.__path=a.__path AND r.cls=m.cls AND r.attr=a.attr WHERE a.__path=x.__path AND a.method=m.id AND a.name=x.text)))")
 add('check_compile_callbacks_no_runtime_authority','STEP-026','bad_callback_nodes',"NOT EXISTS(SELECT 1 FROM bad_callback_nodes b WHERE b.__path=n.__path AND b.keyword=n.keyword AND (b.__start<n.__start OR b.__start=n.__start AND b.__end>n.__end))", "n.keyword||':'||n.id")
 stages.extend([
 S('raw_parameters',"SELECT DISTINCT p.__path,p.fn,p.name FROM parameters p WHERE p.name IN ('raw','raw_rows','rows') OR EXISTS(SELECT 1 FROM descendants d JOIN nodes n ON n.__path=d.__path AND n.id=d.node WHERE d.__path=p.__path AND d.root=p.annotation AND ((n.kind='identifier' AND n.text='RawResult') OR (n.kind IN ('string','concatenated_string') AND instr(n.text,'RawResult')>0)))",('__path','fn','name')),
 S('raw_shapers',"""WITH RECURSIVE shaping(__path,name) AS (
 SELECT DISTINCT f.__path,f.name FROM functions f JOIN raw_parameters p ON p.__path=f.__path AND p.fn=f.id JOIN descendants d ON d.__path=f.__path AND d.root=f.id JOIN nodes n ON n.__path=d.__path AND n.id=d.node
 WHERE (n.kind IN ('for_statement','for_in_clause') AND EXISTS(SELECT 1 FROM descendants it JOIN nodes r ON r.__path=it.__path AND r.id=it.node WHERE it.__path=n.__path AND it.root=n.right_kind||':'||n.right_start||':'||n.right_end AND r.kind='identifier' AND r.text=p.name))
 OR EXISTS(SELECT 1 FROM calls c JOIN arguments a ON a.__path=c.__path AND a.call=c.id AND a.position=0 JOIN descendants v ON v.__path=a.__path AND v.root=a.id JOIN nodes r ON r.__path=v.__path AND r.id=v.node WHERE c.__path=n.__path AND c.id=n.id AND c.full_callee IN ('list','tuple','dict','set','sorted') AND r.kind='identifier' AND r.text=p.name)
 UNION SELECT f.__path,f.name FROM functions f JOIN raw_parameters p ON p.__path=f.__path AND p.fn=f.id JOIN descendants d ON d.__path=f.__path AND d.root=f.id JOIN calls c ON c.__path=d.__path AND c.id=d.node JOIN shaping s ON s.__path=c.__path AND s.name=c.full_callee
 ) SELECT * FROM shaping""",('__path','name')),
 S('shaping_methods',"SELECT DISTINCT m.__path,m.cls,m.id FROM methods m WHERE m.name='materialize' AND (EXISTS(SELECT 1 FROM raw_shapers s WHERE s.__path=m.__path AND s.name=m.name) OR EXISTS(SELECT 1 FROM descendants d JOIN calls c ON c.__path=d.__path AND c.id=d.node JOIN raw_shapers s ON s.__path=c.__path AND s.name=c.full_callee WHERE d.__path=m.__path AND d.root=m.id))",('__path','cls')),
 S('materializer_values',"SELECT n.__path,n.id,c.id AS cls FROM nodes n JOIN classes c ON c.__path=n.__path WHERE n.kind='identifier' AND n.text=c.name OR n.kind='call' AND EXISTS(SELECT 1 FROM calls call WHERE call.__path=n.__path AND call.id=n.id AND call.callee=c.name)",('__path','id')),
 S('column_derived',"""WITH RECURSIVE derived(__path,id) AS (
 SELECT __path,id FROM calls WHERE full_callee LIKE '%cte_output_columns'
 UNION SELECT c.__path,e.src FROM derived c JOIN edges e ON e.__path=c.__path AND e.dst=c.id
 UNION SELECT n.__path,n.id FROM derived c JOIN local_bindings b ON b.__path=c.__path AND b.value=c.id JOIN ownership o ON o.__path=b.__path AND o.fn=b.owner JOIN nodes n ON n.__path=o.__path AND n.id=o.id AND n.kind='identifier' AND n.text=b.name
 ) SELECT * FROM derived""",('__path','id')),
 ])
 add('check_cte_type_hints_not_inferred_from_column_names','STEP-040','calls',"n.callee='SqlColumnTypeHints' AND EXISTS(SELECT 1 FROM method_descendants md WHERE md.__path=n.__path AND md.node=n.id AND md.name='cte_column_type_hints') AND (EXISTS(SELECT 1 FROM arguments a JOIN column_derived d ON d.__path=a.__path AND d.id=a.id WHERE a.__path=n.__path AND a.call=n.id) OR EXISTS(SELECT 1 FROM keywords k JOIN column_derived d ON d.__path=k.__path AND d.id=k.value WHERE k.__path=n.__path AND k.call=n.id) OR EXISTS(SELECT 1 FROM descendants d JOIN nodes x ON x.__path=d.__path AND x.id=d.node WHERE d.__path=n.__path AND d.root=n.id AND x.kind IN ('list_comprehension','set_comprehension','dictionary_comprehension','generator_expression','conditional_expression')))")
 add('check_row_materializer_raw_row_shaping_guardrail','STEP-039','nodes',"EXISTS(SELECT 1 FROM keywords k JOIN materializer_values v ON v.__path=k.__path AND v.id=k.value JOIN shaping_methods s ON s.__path=v.__path AND s.cls=v.cls WHERE k.__path=n.__path AND k.value=n.id AND k.name='row_materializer')")
 # Literal-payload traversal walks call arguments and container values, excluding dict keys.
 stages.extend([
 S('payload_roots',"SELECT k.* FROM keywords k JOIN calls c ON c.__path=k.__path AND c.id=k.call WHERE c.callee='ResultMaterializationSpec' AND (k.name='literal_payload' OR k.name='terminal_literal' AND NOT EXISTS(SELECT 1 FROM keywords other WHERE other.__path=k.__path AND other.call=k.call AND other.name='literal_payload'))",('__path','value')),
 S('payload_values',"""WITH RECURSIVE values_in_payload(__path,root,id) AS (
 SELECT __path,value,value FROM payload_roots
 UNION SELECT v.__path,v.root,e.dst FROM values_in_payload v JOIN nodes n ON n.__path=v.__path AND n.id=v.id JOIN edges e ON e.__path=n.__path AND e.src=n.id WHERE n.kind IN ('tuple','list','set','dictionary','argument_list','keyword_argument')
 UNION SELECT v.__path,v.root,k.value FROM values_in_payload v JOIN calls c ON c.__path=v.__path AND c.id=v.id JOIN keywords k ON k.__path=c.__path AND k.call=c.id
 UNION SELECT v.__path,v.root,a.id FROM values_in_payload v JOIN calls c ON c.__path=v.__path AND c.id=v.id JOIN arguments a ON a.__path=c.__path AND a.call=c.id
 UNION SELECT v.__path,v.root,f.child FROM values_in_payload v JOIN nodes n ON n.__path=v.__path AND n.id=v.id AND n.kind='pair' JOIN fields f ON f.__path=n.__path AND f.owner=n.id AND f.role='value'
 ) SELECT * FROM values_in_payload""",('__path','id')),
 ])
 add('check_literal_payload_materializer_dispatch_guardrail','STEP-039','nodes',"EXISTS(SELECT 1 FROM payload_values p JOIN materializer_values v ON v.__path=p.__path AND v.id=p.id JOIN shaping_methods s ON s.__path=v.__path AND s.cls=v.cls WHERE p.__path=n.__path AND p.id=n.id)")
 # Authored enum symbols retain the source's default audit-debt classification.
 non_audit={symbol for symbol,fate in m.POST_OP_FATE_OVERRIDES.items() if fate is not m.PostOpFate.AUDIT_REQUIRED}
 stages.append(S('postop_symbols',"""SELECT DISTINCT post.__path,post.call,a.text AS symbol FROM keywords post JOIN descendants d ON d.__path=post.__path AND d.root=post.value JOIN calls b ON b.__path=d.__path AND b.id=d.node JOIN arguments arg ON arg.__path=b.__path AND arg.call=b.id JOIN attributes a ON a.__path=arg.__path AND a.id=arg.id WHERE post.name='closed_post_op' AND b.callee='runtime_builder_post_op' AND (arg.position=0 AND a.object_text='RuntimeBuilderKind' OR arg.position=1 AND a.object_text='RuntimeBuilderName')
 UNION SELECT DISTINCT post.__path,post.call,a.text FROM keywords post JOIN descendants d ON d.__path=post.__path AND d.root=post.value JOIN calls b ON b.__path=d.__path AND b.id=d.node JOIN keywords k ON k.__path=b.__path AND k.call=b.id JOIN attributes a ON a.__path=k.__path AND a.id=k.value WHERE post.name='closed_post_op' AND b.callee IN ('runtime_builder_post_op','RuntimeBuilderSpec') AND (k.name='kind' AND a.object_text='RuntimeBuilderKind' OR k.name='builder_name' AND a.object_text='RuntimeBuilderName')""",('__path','call')))
 add('check_runtime_refs_without_sql_grammar_guardrail','STEP-033','nodes',"EXISTS(SELECT 1 FROM compiled c JOIN nonempty_keywords k ON k.__path=c.__path AND k.call=c.id AND k.name='runtime_ref_ids' WHERE c.__path=n.__path AND k.value=n.id AND c.grammarless AND (NOT EXISTS(SELECT 1 FROM postop_symbols s WHERE s.__path=c.__path AND s.call=c.id) OR EXISTS(SELECT 1 FROM postop_symbols s WHERE s.__path=c.__path AND s.call=c.id AND s.symbol IN "+literals(non_audit)+")))")
 stages.extend([
 S('callable_edges',"""SELECT a.__path,a.object AS src,a.id AS dst FROM attributes a
 UNION SELECT a.__path,a.attribute_kind||':'||a.attribute_start||':'||a.attribute_end,a.id FROM attributes a
 UNION SELECT c.__path,c.callee_id,c.id FROM calls c
 UNION SELECT c.__path,a.id,c.id FROM calls c JOIN arguments a ON a.__path=c.__path AND a.call=c.id
 UNION SELECT c.__path,k.value,c.id FROM calls c JOIN keywords k ON k.__path=c.__path AND k.call=c.id
 UNION SELECT n.__path,c.id,n.id FROM nodes n JOIN nodes c ON c.__path=n.__path AND c.parent=n.id AND c.ordinal=0 WHERE n.kind IN ('subscript','parenthesized_expression')""",('__path','src')),
 S('user_callable',"WITH RECURSIVE shaped(__path,id) AS (SELECT __path,id FROM nodes WHERE kind='lambda' OR kind='identifier' AND rx_matches(lower(text),"+q('|'.join(re.escape(v) for v in sorted(m.USER_CALLABLE_ESCAPE_NAMES)))+") UNION SELECT s.__path,e.dst FROM shaped s JOIN callable_edges e ON e.__path=s.__path AND e.src=s.id) SELECT * FROM shaped",('__path','id')),
 S('direct_callable',"""WITH RECURSIVE direct(__path,id) AS (
 SELECT n.__path,n.id FROM nodes n WHERE n.kind='lambda' OR n.kind='identifier' AND (EXISTS(SELECT 1 FROM functions f WHERE f.__path=n.__path AND f.name=n.text) OR EXISTS(SELECT 1 FROM assignments a WHERE a.__path=n.__path AND a.target_text=n.text AND a.expr_kind='lambda'))
 UNION SELECT __path,id FROM calls WHERE full_callee IN ('partial','functools.partial')
 UNION SELECT s.__path,e.dst FROM direct s JOIN expression_edges e ON e.__path=s.__path AND e.src=s.id JOIN nodes n ON n.__path=e.__path AND n.id=e.dst WHERE n.kind!='identifier') SELECT * FROM direct""",('__path','id')),
 S('mutable_values',"WITH RECURSIVE mutable(__path,id) AS (SELECT __path,id FROM nodes WHERE kind IN ('list','dictionary','set','list_comprehension','dictionary_comprehension','set_comprehension') UNION SELECT m.__path,n.parent FROM mutable m JOIN nodes n ON n.__path=m.__path AND n.id=m.id JOIN nodes p ON p.__path=n.__path AND p.id=n.parent AND p.kind='tuple') SELECT * FROM mutable",('__path','id')),
 S('builder_calls',"SELECT * FROM calls WHERE callee IN "+literals(m.RUNTIME_BUILDER_CALL_NAMES),('__path','id')),
 S('literal_kwargs',"SELECT k.* FROM keywords k JOIN builder_calls c ON c.__path=k.__path AND c.id=k.call AND c.callee='RuntimeBuilderSpec' WHERE k.name='literal_kwargs'",('__path','value')),
 S('literal_pairs',"""SELECT k.__path,k.value AS root,key.text AS key,value.id AS value FROM literal_kwargs k JOIN nodes pair ON pair.__path=k.__path AND pair.parent=k.value JOIN nodes key ON key.__path=pair.__path AND key.parent=pair.id AND key.ordinal=0 JOIN nodes value ON value.__path=pair.__path AND value.parent=pair.id AND value.ordinal=1 WHERE pair.kind IN ('pair','tuple','list') AND key.kind IN ('string','concatenated_string')""",('__path','root')),
 ])
 escape=q('|'.join(re.escape(v) for v in sorted(m.RUNTIME_BUILDER_ESCAPE_NAMES)))
 keyword_escape=q('|'.join(re.escape(v) for v in sorted(m.USER_CALLABLE_ESCAPE_NAMES|m.RUNTIME_BUILDER_INDIRECT_CALLABLE_ESCAPE_NAMES|m.RUNTIME_BUILDER_ESCAPE_NAMES)))
 add('check_closed_post_op_runtime_builder_no_callable_escape_hatches','STEP-029','assignments',"n.target_kind IN ('identifier','attribute') AND rx_matches(lower(n.target_text),'runtime') AND rx_matches(lower(n.target_text),'callback|callable|effect|function|hook')")
 add('check_closed_post_op_runtime_builder_no_callable_escape_hatches','STEP-029','assignments',"EXISTS(SELECT 1 FROM classes c WHERE c.__path=n.__path AND c.name='RuntimeBuilderKind' AND EXISTS(SELECT 1 FROM nodes expr WHERE expr.__path=n.__path AND expr.id=n.parent AND expr.kind='expression_statement' AND expr.parent=c.body_kind||':'||c.body_start||':'||c.body_end)) AND (n.target_kind='identifier' AND rx_matches(lower(n.target_text),"+escape+") OR n.expr_kind IN ('string','concatenated_string') AND rx_matches(lower(n.expr_text),"+escape+"))", "'kind:'||n.id")
 add('check_closed_post_op_runtime_builder_no_callable_escape_hatches','STEP-029','calls',"rx_matches(lower(n.full_callee),'runtime') AND rx_matches(lower(n.full_callee),'register') AND rx_matches(lower(n.full_callee),'callback|callable|effect|function|hook|processor|transform')", "'registry:'||n.id")
 add('check_closed_post_op_runtime_builder_no_callable_escape_hatches','STEP-029','builder_calls',"EXISTS(SELECT 1 FROM arguments a WHERE a.__path=n.__path AND a.call=n.id AND ((n.callee='runtime_builder_post_op' AND a.position=0 AND a.kind IN ('identifier','attribute') OR n.callee IN ('runtime_builder_post_op','runtime_op_post_op') AND a.position=CASE WHEN n.callee='runtime_builder_post_op' THEN 1 ELSE 0 END AND a.kind IN ('string','concatenated_string')) AND rx_matches(lower(a.text),"+escape+"))) OR EXISTS(SELECT 1 FROM keywords k WHERE k.__path=n.__path AND k.call=n.id AND k.name IN ('kind','builder_name') AND rx_matches(lower(k.text),"+escape+") AND ((k.name='kind' AND k.kind IN ('identifier','attribute') AND n.callee!='runtime_op_post_op') OR k.name='builder_name' AND k.kind IN ('string','concatenated_string')))", "'escape-name:'||n.id")
 add('check_closed_post_op_runtime_builder_no_callable_escape_hatches','STEP-029','nodes',"EXISTS(SELECT 1 FROM keywords k JOIN builder_calls c ON c.__path=k.__path AND c.id=k.call WHERE k.__path=n.__path AND k.value=n.id AND (lower(k.name) IN "+literals(m.RUNTIME_BUILDER_OPAQUE_CARRIER_NAMES)+" OR rx_matches(lower(k.name),"+keyword_escape+") OR EXISTS(SELECT 1 FROM user_callable x WHERE x.__path=k.__path AND x.id=k.value) OR EXISTS(SELECT 1 FROM direct_callable x WHERE x.__path=k.__path AND x.id=k.value) OR k.name!='literal_kwargs' AND EXISTS(SELECT 1 FROM mutable_values x WHERE x.__path=k.__path AND x.id=k.value)))", "'keyword:'||n.id")
 add('check_closed_post_op_runtime_builder_no_callable_escape_hatches','STEP-029','nodes',"EXISTS(SELECT 1 FROM literal_kwargs k WHERE k.__path=n.__path AND k.value=n.id AND (EXISTS(SELECT 1 FROM literal_pairs p WHERE p.__path=k.__path AND p.root=k.value AND (rx_matches(lower(p.key),"+escape+") OR lower(trim(p.key,char(34)||char(39))) IN "+literals(m.RUNTIME_BUILDER_OPAQUE_CARRIER_NAMES)+" OR EXISTS(SELECT 1 FROM user_callable u WHERE u.__path=p.__path AND u.id=p.value))) OR EXISTS(SELECT 1 FROM user_callable u WHERE u.__path=k.__path AND u.id=k.value)))", "'literal-escape:'||n.id")
 add('check_closed_post_op_runtime_builder_no_callable_escape_hatches','STEP-029','nodes',"EXISTS(SELECT 1 FROM literal_kwargs k WHERE k.__path=n.__path AND k.value=n.id AND (EXISTS(SELECT 1 FROM literal_pairs p JOIN direct_callable u ON u.__path=p.__path AND u.id=p.value WHERE p.__path=k.__path AND p.root=k.value) OR EXISTS(SELECT 1 FROM direct_callable u WHERE u.__path=k.__path AND u.id=k.value)))", "'literal-callable:'||n.id")
 add('check_closed_post_op_runtime_builder_no_callable_escape_hatches','STEP-029','nodes',"EXISTS(SELECT 1 FROM literal_kwargs k JOIN mutable_values u ON u.__path=k.__path AND u.id=k.value WHERE k.__path=n.__path AND k.value=n.id)", "'literal-mutable:'||n.id")
 stages.extend([
 S('table_problems',"""SELECT * FROM (
 SELECT r.__path,r.cls,r.name,r.id,n.__start,n.__end,row_number() OVER(PARTITION BY r.__path,r.cls,r.name ORDER BY n.__start) AS selected
 FROM method_returns r JOIN nodes n ON n.__path=r.__path AND n.id=r.id JOIN calls c ON c.__path=r.__path AND c.id=r.value JOIN arguments a ON a.__path=c.__path AND a.call=c.id AND a.position=0
 WHERE r.name IN ('tables_read','tables_written') AND c.full_callee='frozenset' AND (a.kind NOT IN ('list','set','tuple') OR EXISTS(SELECT 1 FROM nodes s WHERE s.__path=a.__path AND s.parent=a.id AND s.kind IN ('string','concatenated_string') AND trim(s.text,char(34)||char(39))!='*'))
 ) WHERE selected=1
 UNION SELECT m.__path,m.cls,m.name,m.id,m.__start,m.__end,1 FROM first_methods m WHERE m.name IN ('tables_read','tables_written') AND NOT EXISTS(SELECT 1 FROM empty_tables e WHERE e.__path=m.__path AND e.method=m.id)
 AND NOT EXISTS(SELECT 1 FROM method_returns r JOIN calls c ON c.__path=r.__path AND c.id=r.value JOIN arguments a ON a.__path=c.__path AND a.call=c.id AND a.position=0 WHERE r.__path=m.__path AND r.method=m.id AND c.full_callee='frozenset' AND (a.kind NOT IN ('list','set','tuple') OR EXISTS(SELECT 1 FROM nodes s WHERE s.__path=a.__path AND s.parent=a.id AND s.kind IN ('string','concatenated_string') AND trim(s.text,char(34)||char(39))!='*')))""",('__path','cls','name')),
 S('static_exception_seeds',"""SELECT c.__path,c.id FROM calls c JOIN arguments a ON a.__path=c.__path AND a.call=c.id AND a.position=0 WHERE c.full_callee LIKE '%static_exception_post_op' AND a.text IN ('RuntimeError','TypeError','NotImplementedError')
 UNION SELECT c.__path,c.id FROM calls c JOIN arguments a ON a.__path=c.__path AND a.call=c.id AND a.position=0 JOIN calls spec ON spec.__path=a.__path AND spec.id=a.id WHERE c.full_callee LIKE '%CompiledPostOp.assert_result' AND NOT EXISTS(SELECT 1 FROM keywords k WHERE k.__path=spec.__path AND k.call=spec.id AND k.name='static_exception' AND k.kind IN ('identifier','attribute') AND k.text NOT IN ('RuntimeError','TypeError','NotImplementedError'))
 UNION SELECT c.__path,c.id FROM calls c JOIN keywords a ON a.__path=c.__path AND a.call=c.id AND a.name='spec' JOIN calls spec ON spec.__path=a.__path AND spec.id=a.value WHERE c.full_callee LIKE '%CompiledPostOp.assert_result' AND NOT EXISTS(SELECT 1 FROM arguments pos WHERE pos.__path=c.__path AND pos.call=c.id) AND NOT EXISTS(SELECT 1 FROM keywords k WHERE k.__path=spec.__path AND k.call=spec.id AND k.name='static_exception' AND k.kind IN ('identifier','attribute') AND k.text NOT IN ('RuntimeError','TypeError','NotImplementedError'))""",('__path','id')),
 S('static_exception_functions',"""WITH RECURSIVE bad(__path,name) AS (
 SELECT DISTINCT f.__path,f.name FROM functions f JOIN descendants d ON d.__path=f.__path AND d.root=f.id JOIN calls c ON c.__path=d.__path AND c.id=d.node AND c.callee='ResultAssertionSpec' JOIN keywords k ON k.__path=c.__path AND k.call=c.id AND k.name='static_exception' AND k.kind='identifier' JOIN parameters p ON p.__path=f.__path AND p.fn=f.id AND p.name=k.text
 UNION SELECT f.__path,f.name FROM functions f JOIN returns r ON r.__path=f.__path AND r.fn=f.id JOIN static_exception_seeds b ON b.__path=r.__path AND b.id=r.value
 UNION SELECT f.__path,f.name FROM functions f JOIN returns r ON r.__path=f.__path AND r.fn=f.id JOIN calls c ON c.__path=r.__path AND c.id=r.value JOIN bad b ON b.__path=c.__path AND b.name=c.callee) SELECT * FROM bad""",('__path','name')),
 S('materialization_functions',"SELECT DISTINCT f.__path,f.id FROM functions f JOIN descendants d ON d.__path=f.__path AND d.root=f.id JOIN calls c ON c.__path=d.__path AND c.id=d.node WHERE c.callee='ResultMaterializationSpec' OR c.full_callee LIKE '%CompiledPostOp.materialize' OR c.callee='CompiledStep' AND EXISTS(SELECT 1 FROM keywords k WHERE k.__path=c.__path AND k.call=c.id AND k.name IN ('post_op','closed_post_op','post_op_with_refs'))",('__path','id')),
 ])
 add('check_step_role_contract_guardrails','STEP-045','table_problems',"EXISTS(SELECT 1 FROM roles r WHERE r.__path=n.__path AND r.cls=n.cls AND r.role='VIRTUAL') AND (n.name='tables_read' OR NOT EXISTS(SELECT 1 FROM table_problems r WHERE r.__path=n.__path AND r.cls=n.cls AND r.name='tables_read'))")
 add('check_compiled_result_builders_do_not_fail_closed','STEP-041','nodes',"n.kind='raise_statement' AND EXISTS(SELECT 1 FROM ownership o JOIN materialization_functions f ON f.__path=o.__path AND f.id=o.fn WHERE o.__path=n.__path AND o.id=n.id) AND EXISTS(SELECT 1 FROM nodes e WHERE e.__path=n.__path AND e.parent=n.id AND (e.kind='identifier' AND e.text IN ('RuntimeError','TypeError','NotImplementedError') OR EXISTS(SELECT 1 FROM calls c WHERE c.__path=e.__path AND c.id=e.id AND c.full_callee IN ('RuntimeError','TypeError','NotImplementedError'))))")
 add('check_compiled_result_builders_do_not_fail_closed','STEP-041','nodes',"EXISTS(SELECT 1 FROM keywords k JOIN compiled c ON c.__path=k.__path AND c.id=k.call JOIN ownership o ON o.__path=c.__path AND o.id=c.id JOIN materialization_functions f ON f.__path=o.__path AND f.id=o.fn WHERE k.__path=n.__path AND k.value=n.id AND k.name='closed_post_op' AND (EXISTS(SELECT 1 FROM static_exception_seeds s WHERE s.__path=k.__path AND s.id=k.value) OR EXISTS(SELECT 1 FROM calls helper JOIN static_exception_functions f ON f.__path=helper.__path AND f.name=helper.callee WHERE helper.__path=k.__path AND helper.id=k.value)))")
 # Mutations in nested compile callbacks retain the capture's enclosing compile.
 stages.append(S('postop_callbacks',"SELECT m.__path,m.id AS method,m.cls,f.id AS callback FROM step_methods m JOIN functions f ON f.__path=m.__path AND f.__start>m.__start AND f.__end<=m.__end WHERE m.name='compile' AND f.name NOT IN ('on_commit','on_rollback')",('__path','callback')))
 add('check_postop_no_resolved_data_manipulation','STEP-019','nodes',"EXISTS(SELECT 1 FROM postop_callbacks p JOIN descendants d ON d.__path=p.__path AND d.root=p.callback WHERE p.__path=n.__path AND d.node=n.id AND (EXISTS(SELECT 1 FROM assignments a JOIN attributes attr ON attr.__path=a.__path AND attr.id=a.target JOIN compile_aliases alias ON alias.__path=attr.__path AND alias.method=p.method AND alias.name=attr.object_text WHERE a.__path=n.__path AND a.id=n.id) OR EXISTS(SELECT 1 FROM calls c JOIN attributes attr ON attr.__path=c.__path AND attr.id=c.callee_id JOIN compile_aliases alias ON alias.__path=attr.__path AND alias.method=p.method AND alias.name=attr.object_text WHERE c.__path=n.__path AND c.id=n.id AND c.callee IN "+literals(m.POSTOP_MANIPULATION_METHODS)+") OR EXISTS(SELECT 1 FROM calls c JOIN arguments a ON a.__path=c.__path AND a.call=c.id AND a.position=0 JOIN compile_aliases alias ON alias.__path=a.__path AND alias.method=p.method AND alias.name=a.text WHERE c.__path=n.__path AND c.id=n.id AND c.callee='__setattr__' AND c.callee_kind='attribute' AND a.kind='identifier')))")
 stages.extend([
 S('legacy_postops',"SELECT k.* FROM keywords k JOIN compiled c ON c.__path=k.__path AND c.id=k.call WHERE k.name IN ('post_op','post_op_with_refs') AND (EXISTS(SELECT 1 FROM relaxed r WHERE r.__path=k.__path) OR EXISTS(SELECT 1 FROM assignments a JOIN nodes p ON p.__path=a.__path AND p.id=a.parent JOIN nodes root ON root.__path=p.__path AND root.id=p.parent AND root.kind='module' WHERE a.__path=k.__path AND a.target_text='CLOSED_POST_OP_STRICT' AND a.expr_kind='true' AND a.type_start IS NULL))",('__path','value')),
 S('legacy_callbacks',"SELECT k.__path,k.keyword,f.id FROM legacy_postops k JOIN functions f ON f.__path=k.__path AND f.name=k.text WHERE k.kind='identifier' AND NOT EXISTS(SELECT 1 FROM functions newer WHERE newer.__path=f.__path AND newer.name=f.name AND newer.__start>f.__start)",('__path','id')),
 ])
 add('check_legacy_postop_user_callable_escape_hatches','STEP-028','nodes',"EXISTS(SELECT 1 FROM legacy_postops k WHERE k.__path=n.__path AND k.value=n.id AND (EXISTS(SELECT 1 FROM calls c WHERE c.__path=k.__path AND c.id=k.value AND (c.full_callee='partial' AND EXISTS(SELECT 1 FROM user_callable u WHERE u.__path=c.__path AND u.id=c.id) OR c.full_callee!='partial' AND k.name='post_op_with_refs')) OR k.kind!='call' AND EXISTS(SELECT 1 FROM user_callable u WHERE u.__path=k.__path AND u.id=k.value)))", "'value:'||n.id")
 add('check_legacy_postop_user_callable_escape_hatches','STEP-028','calls',"EXISTS(SELECT 1 FROM legacy_callbacks k JOIN descendants d ON d.__path=k.__path AND d.root=k.id JOIN user_callable u ON u.__path=d.__path AND u.id=d.node WHERE k.__path=n.__path AND d.node=n.id)", "'callback:'||n.id")
