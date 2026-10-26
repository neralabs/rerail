# SPDX-License-Identifier: Apache-2.0
"""Generic CST relations used by authored sixth-batch graph policies."""
from .common import *

FIELDS=('name','left','right','value','type','function','arguments','object','attribute','body','parameters','return_type','condition','subject','consequence','alias','module_name')
def ast_inputs():
 rule=A(P('NODE'),optional(I(P('PARENT'),'neighbor')),*[optional(F(f,P(f.upper()))) for f in FIELDS])
 project={'kind':'capture.NODE.kind','text':'capture.NODE','line':'capture.NODE.startLine',
  'parent_start':'capture.PARENT.startByte','parent_end':'capture.PARENT.endByte','parent_kind':'capture.PARENT.kind',
  'ancestor_start':'capture.PARENT.startByte','ancestor_end':'capture.PARENT.endByte','ancestor_kind':'capture.PARENT.kind'}
 for f in FIELDS:
  project[f+'_start']='capture.'+f.upper()+'.startByte'
  project[f+'_end']='capture.'+f.upper()+'.endByte'
  project[f+'_kind']='capture.'+f.upper()+'.kind'
  project[f+'_source']='capture.'+f.upper()
 return {'raw_nodes':E(rule,project)}

def ast_stages():
 return [S('nodes',"""SELECT *,kind||':'||__start||':'||__end AS id,
 parent_kind||':'||parent_start||':'||parent_end AS parent,
 ancestor_kind||':'||ancestor_start||':'||ancestor_end AS edge_parent,
 row_number() OVER(PARTITION BY __path,parent_start,parent_end,parent_kind ORDER BY __start,__end,kind)-1 AS ordinal
 FROM raw_nodes""",('__path','id'),('__path','parent'),('__path','kind'),('__path','__start','__end')),
 S('edges','SELECT __path,edge_parent AS src,id AS dst,ordinal FROM nodes WHERE edge_parent IS NOT NULL',('__path','src'),('__path','dst')),
 S('fields'," UNION ALL ".join(f"SELECT __path,id AS owner,{q(f)} AS role,{f}_kind||':'||{f}_start||':'||{f}_end AS child FROM nodes WHERE {f}_start IS NOT NULL" for f in FIELDS),('__path','owner','role'),('__path','child')),
 S('functions',"SELECT n.*,n.name_source AS name FROM nodes n WHERE n.kind='function_definition'",('__path','id'),('__path','name')),
 S('ownership',"""WITH RECURSIVE owned(__path,id,fn,cls,top_fn) AS (
 SELECT n.__path,n.id,CASE WHEN n.kind='function_definition' THEN n.id END,CASE WHEN n.kind='class_definition' THEN n.id END,CASE WHEN n.kind='function_definition' THEN n.id END FROM nodes n
 LEFT JOIN nodes parent ON parent.__path=n.__path AND parent.id=n.edge_parent WHERE parent.id IS NULL
 UNION ALL SELECT child.__path,child.id,CASE WHEN child.kind='function_definition' THEN child.id ELSE owned.fn END,CASE WHEN child.kind='class_definition' THEN child.id ELSE owned.cls END,CASE WHEN owned.top_fn IS NULL AND child.kind='function_definition' THEN child.id ELSE owned.top_fn END
 FROM owned JOIN edges e ON e.__path=owned.__path AND e.src=owned.id
 JOIN nodes child ON child.__path=e.__path AND child.id=e.dst) SELECT * FROM owned""",('__path','id'),('__path','fn'),('__path','top_fn'))]

def narrow_ast_ordinals(clause,parent_kinds):
 """Number all children of demanded parent kinds; retain other nodes without an ordinal."""
 plan=clause['relational']
 nodes=next(stage for stage in plan['stages'] if stage['name']=='nodes')
 ordinal="row_number() OVER(PARTITION BY __path,parent_start,parent_end,parent_kind ORDER BY __start,__end,kind)-1"
 if ordinal+' AS ordinal' not in nodes['query']:raise ValueError('expected the authored CST ordinal window')
 window=S('node_ordinals','SELECT __path,__start,__end,kind,'+ordinal+' AS ordinal FROM raw_nodes WHERE parent_kind IN '+literals(parent_kinds),('__path','__start','__end','kind'))
 nodes['query']="""SELECT n.*,n.kind||':'||n.__start||':'||n.__end AS id,
 n.parent_kind||':'||n.parent_start||':'||n.parent_end AS parent,
 n.ancestor_kind||':'||n.ancestor_start||':'||n.ancestor_end AS edge_parent,o.ordinal
 FROM raw_nodes n LEFT JOIN node_ordinals o ON o.__path=n.__path AND o.__start=n.__start AND o.__end=n.__end AND o.kind=n.kind"""
 plan['stages'].insert(plan['stages'].index(nodes),window)

def relational(id,stages,query,code='issue',key=('issue','identity')):
 return {'id':id,'relational':{'inputs':ast_inputs(),'stages':[*ast_stages(),*stages],'query':query,'key':list(key),'code':code},
  'violation':{'code':'architecture','message':'Architecture contract violation','anchor':'witness.span'}}

def python_stages():
 """Named expression roles, declarations, and call arguments; no policy semantics."""
 return [
 S('classes',"SELECT n.*,n.name_source AS name FROM nodes n WHERE n.kind='class_definition'",('__path','id'),('__path','name')),
 S('methods',"""SELECT f.*,c.id AS cls,c.name AS class_name FROM functions f JOIN classes c ON c.__path=f.__path
 WHERE f.parent=c.body_kind||':'||c.body_start||':'||c.body_end OR EXISTS(SELECT 1 FROM nodes d WHERE d.__path=f.__path AND d.id=f.parent AND d.kind='decorated_definition' AND d.parent=c.body_kind||':'||c.body_start||':'||c.body_end)""",('__path','cls','name'),('__path','id')),
 S('attributes',"SELECT n.*,n.attribute_source AS attr,n.object_kind||':'||n.object_start||':'||n.object_end AS object,n.object_source AS object_text FROM nodes n WHERE n.kind='attribute'",('__path','id')),
 S('calls',"SELECT n.*,f.id AS callee_id,f.kind AS callee_kind,f.text AS full_callee,coalesce(a.attr,f.text) AS callee,a.object AS receiver FROM nodes n JOIN nodes f ON f.__path=n.__path AND f.id=n.function_kind||':'||n.function_start||':'||n.function_end LEFT JOIN attributes a ON a.__path=f.__path AND a.id=f.id WHERE n.kind='call'",('__path','id'),('__path','callee')),
 S('arguments',"SELECT n.*,c.id AS call,row_number() OVER(PARTITION BY c.__path,c.id ORDER BY n.__start)-1 AS position FROM calls c JOIN nodes n ON n.__path=c.__path AND n.parent=c.arguments_kind||':'||c.arguments_start||':'||c.arguments_end WHERE n.kind NOT IN ('comment','keyword_argument','list_splat','dictionary_splat')",('__path','call','position')),
 S('keywords',"SELECT c.__path,c.id AS call,k.id AS keyword,k.__start AS keyword_start,kn.text AS name,v.id AS value,v.__start,v.__end,v.text,v.kind FROM calls c JOIN nodes k ON k.__path=c.__path AND k.parent=c.arguments_kind||':'||c.arguments_start||':'||c.arguments_end AND k.kind='keyword_argument' JOIN nodes kn ON kn.__path=k.__path AND kn.id=k.name_kind||':'||k.name_start||':'||k.name_end JOIN nodes v ON v.__path=k.__path AND v.id=k.value_kind||':'||k.value_start||':'||k.value_end",('__path','call','name'),('__path','value')),
 S('descendants',"WITH RECURSIVE walk(__path,root,node) AS (SELECT __path,id,id FROM nodes UNION SELECT w.__path,w.root,e.dst FROM walk w JOIN edges e ON e.__path=w.__path AND e.src=w.node) SELECT * FROM walk",('__path','root','node'),('__path','node','root')),
 S('returns',"SELECT n.*,v.id AS value,v.kind AS expr_kind,v.text AS expr_text,o.fn FROM nodes n JOIN nodes v ON v.__path=n.__path AND v.parent=n.id LEFT JOIN ownership o ON o.__path=n.__path AND o.id=n.id WHERE n.kind='return_statement'",('__path','fn'),('__path','value')),
 S('assignments',"SELECT n.*,l.id AS target,l.kind AS target_kind,l.text AS target_text,r.id AS value,r.kind AS expr_kind,r.text AS expr_text,o.fn FROM nodes n JOIN nodes l ON l.__path=n.__path AND l.id=n.left_kind||':'||n.left_start||':'||n.left_end LEFT JOIN nodes r ON r.__path=n.__path AND r.id=n.right_kind||':'||n.right_start||':'||n.right_end LEFT JOIN ownership o ON o.__path=n.__path AND o.id=n.id WHERE n.kind='assignment'",('__path','fn','target_text'),('__path','id'),('__path','value')),
 S('parameters',"""SELECT f.__path,f.id AS fn,p.id,p.kind,p.text,n.text AS name,p.type_kind||':'||p.type_start||':'||p.type_end AS annotation,
 row_number() OVER(PARTITION BY f.__path,f.id ORDER BY p.__start)-1 AS position FROM functions f JOIN nodes p ON p.__path=f.__path AND p.parent=f.parameters_kind||':'||f.parameters_start||':'||f.parameters_end JOIN nodes n ON n.__path=p.__path AND (n.id=p.id AND p.kind='identifier' OR n.parent=p.id AND n.ordinal=0 AND n.kind='identifier')
 WHERE p.kind IN ('identifier','typed_parameter','default_parameter','typed_default_parameter')""",('__path','fn','name'))]

def grouped_relations(prefix,stages,rows,full_entries,extra_inputs=None,production_rows=None,row_groups=None,shared_sql=False):
 """Prune detector dependencies, then group within the declared stage bound.

 Rows contain (entry-point tuple, source query, full-analyzer query).
 """
 all_stages=ast_stages()+stages;by_name={stage['name']:stage for stage in all_stages}
 def dependencies(sql):
  needed=set();pending=[sql if isinstance(sql,str) else json.dumps(sql)]
  while pending:
   for name in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',pending.pop()):
    if name in by_name and name not in needed:needed.add(name);pending.append(by_name[name]['query'] if isinstance(by_name[name]['query'],str) else json.dumps(by_name[name]['query']))
  return needed
 groups=[]
 for row in rows:
  tag=(row_groups or {}).get(row[2],'default')
  needed=dependencies(row[1]+' '+row[2])
  if len(needed)>32:raise ValueError(f'{row[0]} needs {len(needed)} stages')
  group=next((g for g in groups if g['tag']==tag and (shared_sql or len(g['names']|needed)<=32 and sum(len(r[2].encode())+11 for r in g['rows'])+len(row[2].encode())<(13000 if production_rows else 16000))),None)
  if group is None:group={'rows':[],'names':set(),'tag':tag};groups.append(group)
  group['names']|=needed;group['rows'].append(row)
 result=[]
 for i,group in enumerate(groups):
  entries=sorted({entry for es,_,_ in group['rows'] for entry in es})
  def query_for(selected):
   full=' UNION ALL '.join(sql for _,_,sql in selected)
   queries={**{e:full for e in full_entries},**{entry:' UNION ALL '.join(sql for es,sql,_ in selected if entry in es) for entry in sorted({e for es,_,_ in selected for e in es})}}
   if production_rows is not None:
    queries['all']=choice('oracleAnalyzer',full,' UNION ALL '.join(production_rows[sql] for _,_,sql in selected))
   return {'$option':'entryPoint','cases':queries,'default':'SELECT * FROM ('+full+') WHERE 0'}
  # Share source inputs and indexed SQL tables in one ordinary relational clause.
  # Split only diagnostic SELECTs to keep each authored SQL statement bounded.
  # SQLite owns every intermediate relation; there is no runtime artifact cache.
  outputs=[]
  if shared_sql:
   packets=[]
   for row in group['rows']:
    size=max(len(row[1].encode()),len(row[2].encode()),len(production_rows[row[2]].encode()) if production_rows else 0)+32
    if not packets or packets[-1][1]+size>12000:packets.append(([],0))
    packet,total=packets[-1];packet.append(row);packets[-1]=(packet,total+size)
   outputs=[S('diagnostics_'+str(j),query_for(packet)) for j,(packet,_) in enumerate(packets)]
   query=' UNION ALL '.join('SELECT * FROM '+stage['name'] for stage in outputs)
  else:query=query_for(group['rows'])
  clause=relational(prefix+'-'+str(i+1),[],query,key=('detector','issue','identity'))
  import copy
  clause['relational']['stages']=[copy.deepcopy(s) for s in all_stages if s['name'] in group['names']]+outputs
  clause['relational']['inputs']['raw_nodes']['evidence']['query']['ast']['rule']['all'].append(gate([*entries,*full_entries]))
  reads=set(re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',json.dumps(query)+' '+' '.join(s['query'] if isinstance(s['query'],str) else json.dumps(s['query']) for s in clause['relational']['stages'])))
  clause['relational']['inputs'].update({name:copy.deepcopy(value) for name,value in (extra_inputs or {}).items() if name in reads})
  if row_groups is not None:clause['_authoring_group']=group['tag']
  result.append(clause)
 return result

def trim_ast_projection(clause):
 """Avoid collecting child fields that no selected relational stage reads."""
 plan=clause['relational']
 raw=plan['inputs'].get('raw_nodes')
 if raw is None:return
 other_sql=json.dumps(plan['query'])+' '+' '.join(s['query'] if isinstance(s['query'],str) else json.dumps(s['query']) for s in plan['stages'] if s['name']!='fields')
 used_roles=set(re.findall(r"\brole\s*=\s*'([a-z_]+)'",other_sql))
 if used_roles:
  for stage in plan['stages']:
   if stage['name']=='fields':stage['query']=' UNION ALL '.join(f"SELECT __path,id AS owner,{q(f)} AS role,{f}_kind||':'||{f}_start||':'||{f}_end AS child FROM nodes WHERE {f}_start IS NOT NULL" for f in FIELDS if f in used_roles)
 sql=json.dumps(plan['query'])+' '+' '.join(s['query'] if isinstance(s['query'],str) else json.dumps(s['query']) for s in plan['stages'])
 fields={f for f in FIELDS if any(re.search(r'\b'+re.escape(f+'_'+suffix)+r'\b',sql) for suffix in ['kind','start','end','source'])}
 ast=raw['evidence']['query']['ast']
 def trim(rule):
  if not isinstance(rule,dict):return
  parts=rule.get('all')
  if isinstance(parts,list) and parts and parts[0]==P('NODE'):
   rule['all']=[*parts[2+len(FIELDS):],*parts[:2],*[optional(F(f,P(f.upper()))) for f in FIELDS if f in fields]]
  else:
   for value in rule.values():
    if isinstance(value,dict):trim(value)
    elif isinstance(value,list):
     for item in value:trim(item)
 trim(ast['rule'])
 raw['evidence']['project']={name:source for name,source in raw['evidence']['project'].items()
  if name in {'kind','text','line','parent_start','parent_end','parent_kind','ancestor_start','ancestor_end','ancestor_kind'} or any(name==f+'_'+suffix and re.search(r'\b'+re.escape(name)+r'\b',sql) for f in fields for suffix in ['kind','start','end','source'])}

def typed_input_partitions(clause):
 """Collect independent typed relations within each unchanged witness bound."""
 import copy
 plan=clause['relational'];raw=plan['inputs'].pop('raw_nodes',None)
 if raw is None:return
 leaves=O(*(K(k) for k in ['comment','string_start','string_content','string_end','escape_sequence','line_continuation']))
 identifiers=[A(K('identifier'),RX(pattern)) for pattern in ['^[a-f]','^[g-m]','^[n-r]','^s','^[t-z]','^[A-Z]','^_','^[^a-zA-Z_]']]
 thin=clause['id'].startswith('quality-conditional')
 metadata=['module','class_definition','function_definition','decorated_definition','block','expression_statement','argument_list','keyword_argument','assignment','augmented_assignment','return_statement','if_statement','elif_clause','else_clause','match_statement','case_clause','for_statement'] if thin else []
 kinds=[['call'],['attribute'],['argument_list'],['keyword_argument'],['expression_statement'],['assignment','augmented_assignment','return_statement'],['block'],['type'],['typed_parameter','typed_default_parameter','parameters'],['type_parameter','generic_type'],['class_definition','function_definition','decorated_definition'],['integer','float','true','false','none'],['tuple','list','set','dictionary','pair','pattern_list','tuple_pattern','list_pattern']]
 kinds=[[kind for kind in group if kind not in metadata] for group in kinds]
 kinds=[group for group in kinds if group]
 string_kinds=['string','concatenated_string']
 strings=O(*(K(kind) for kind in string_kinds))
 underscore=RX(r"^[rRuUbBfF]*[\"']{1,3}_")
 lowercase=RX(r"^[rRuUbBfF]*[\"']{1,3}[a-z]")
 # The complement preserves quoted, formatted, concatenated, and Unicode shapes.
 string_groups=[A(strings,underscore),A(strings,N(underscore),lowercase),A(strings,N(O(underscore,lowercase)))]
 partitions=[*[(predicate,['identifier'],False) for predicate in identifiers],*[(predicate,string_kinds,False) for predicate in string_groups],*[(O(*(K(kind) for kind in group)),group,False) for group in kinds],(N(O(K('identifier'),strings,leaves,*(K(kind) for group in kinds for kind in group),*(K(kind) for kind in metadata))),[],False)]
 if metadata:partitions.append((O(*(K(kind) for kind in metadata)),metadata,True))
 possible={'identifier':set(),'string':set(),'concatenated_string':set(),'call':{'function','arguments'},'attribute':{'object','attribute'},'argument_list':set(),'keyword_argument':{'name','value'},'expression_statement':set(),'assignment':{'left','right','type'},'augmented_assignment':{'left','right'},'return_statement':set(),'module':set(),'class_definition':{'name','body'},'function_definition':{'name','parameters','return_type','body'},'decorated_definition':{'definition'},'block':set(),'if_statement':{'condition','consequence'},'elif_clause':{'condition','consequence'},'else_clause':{'body'},'match_statement':{'subject','body'},'case_clause':{'consequence'},'for_statement':{'left','right','body'}}
 columns=['__path','__start','__end',*raw['evidence']['project']]
 selects=[]
 for index,(predicate,selected,thin_text) in enumerate(partitions):
  relation=copy.deepcopy(raw);ast=relation['evidence']['query']['ast'];ast['rule']=A(predicate,N(leaves),ast['rule'])
  allowed=set().union(*(possible[kind] for kind in selected)) if selected and all(kind in possible for kind in selected) else None
  if allowed is not None:
   relation['evidence']['project']={key:value for key,value in relation['evidence']['project'].items() if not any(key.startswith(field+'_') and field not in allowed for field in FIELDS)}
  if thin_text:
   relation['evidence']['project']['text']={'source':'capture.NODE','regex':'^()','group':1}
  plan['inputs']['syntax_'+str(index)]=relation
  actual={'__path','__start','__end',*relation['evidence']['project']}
  selects.append('SELECT '+','.join(column if column in actual else 'NULL AS '+column for column in columns)+' FROM syntax_'+str(index))
 union=' UNION ALL '.join(selects)
 helpers=[]
 if len(union.encode())>12000:
  chunks=[]
  for select in selects:
   if not chunks or len((' UNION ALL '.join([*chunks[-1],select])).encode())>12000:chunks.append([])
   chunks[-1].append(select)
  helpers=[S('syntax_rows_'+str(index),' UNION ALL '.join(chunk)) for index,chunk in enumerate(chunks)]
  union=' UNION ALL '.join('SELECT * FROM '+stage['name'] for stage in helpers)
 for stage in plan['stages']:
  if isinstance(stage.get('query'),str):stage['query']=stage['query'].replace('FROM raw_nodes','FROM ('+union+')')
 plan['stages']=[*helpers,*plan['stages']]
 if len(plan['stages'])>64:raise ValueError(clause['id']+' needs too many materialized syntax stages')

def batched_reachability(clause,forest=False):
 """Lower explicit graph producers to the generic batched SQLite stage."""
 plan=clause['relational']
 for stage in list(plan.get('stages',[])):
  if stage['name']!='descendants':continue
  original=stage['query']
  if original.startswith('WITH RECURSIVE roots(__path,id) AS ('):
   # Conditional architecture declares roots across the repository.
   roots=original.split(', walk(__path,root,node) AS (',1)[0]+' SELECT __path,id FROM roots'
  elif clause['id'].startswith('quality-contracts'):
   # Every consumer family contributes roots. Comprehension parents and bare
   # comparison operands have no field role and require explicit requests.
   projected={key for value in plan['inputs'].values() for key in value['evidence']['project']}
   seeds=["SELECT __path,id FROM nodes WHERE kind IN ('function_definition','class_definition','for_statement','for_in_clause','assignment','delete_statement','block','if_statement','elif_clause','match_statement')",
    "SELECT __path,parent FROM nodes WHERE kind='for_in_clause' AND parent IS NOT NULL",
    "SELECT child.__path,child.id FROM nodes child JOIN nodes cmp ON cmp.__path=child.__path AND cmp.id=child.parent WHERE cmp.kind='comparison_operator'"]
   seeds += [f"SELECT __path,{f}_kind||':'||{f}_start||':'||{f}_end FROM nodes WHERE {f}_start IS NOT NULL" for f in FIELDS if {f+'_kind',f+'_start',f+'_end'}<=projected]
   if any(s['name']=='arguments' for s in plan['stages']):seeds.append('SELECT __path,id FROM arguments')
   roots=' UNION '.join(seeds)
  elif clause['id'].startswith(('state-signatures','state-flow','state-projection-recovery')):
   # Keep the exact selected-edge forest. Root expansion is independent of
   # traversal through that node on behalf of another root.
   expansion=",kind NOT IN ('module','block','expression_statement','argument_list','decorated_definition','type') AS expand" if 'walk(__path,root,node,expand)' in original else ''
   roots='SELECT __path,id'+expansion+' FROM nodes'
  else:
   # Other linters retain all authored seeds until a smaller producer is proved.
   continue
  edges='SELECT __path,src,dst FROM edges'
  if forest and clause['id'].startswith('quality-conditional'):
   # Keep global declarations, but normalize only the union of demanded CST
   # subtrees. This unlabelled frontier visits overlapping regions just once.
   declared=S('descendant_roots','SELECT __path,id FROM ('+roots+') WHERE __path IS NOT NULL AND id IS NOT NULL',('__path','id'))
   plan['stages'].insert(plan['stages'].index(stage),declared)
   roots='SELECT __path,id FROM descendant_roots'
   edges='WITH RECURSIVE demanded(__path,id) AS (SELECT __path,id FROM descendant_roots UNION SELECT d.__path,e.dst FROM demanded d CROSS JOIN edges e ON e.__path=d.__path AND e.src=d.id) SELECT e.__path,e.src,e.dst FROM demanded d CROSS JOIN edges e ON e.__path=d.__path AND e.src=d.id'
  stage.clear();stage.update(name='descendants',reachability={'roots':roots,'edges':edges,**({'graph':'forest'} if forest else {})})

def defer_node_text(clause):
 """These authored relations carry the node's exact path and byte range.

 Preserve all rule SQL while avoiding full source copies during extraction and
 intermediate materialization. Other text values retain their existing meaning.
 """
 if not clause['id'].startswith('quality-contracts'):return
 plan=clause['relational'];raw=plan['inputs'].get('raw_nodes')
 if raw is None or raw['evidence']['project'].get('text')!='capture.NODE':
  raise ValueError('deferred node text requires an exact source projection')
 del raw['evidence']['project']['text']
 # Explicit producer contracts: text is precisely the source at __start..__end.
 producers={'nodes','functions','classes','methods','attributes','calls','arguments',
  'keywords','returns','assignments','module_functions','last_methods','last_module_functions'}
 for stage in plan['stages']:
  if stage['name'] in producers:
   stage['deferred']={'text':'source_text(__path,__start,__end)'}

def narrow_deferred_windows(clause):
 """Window coroutines otherwise compute all SELECT-star columns before outer pruning."""
 if not clause['id'].startswith('quality-contracts'):return
 plan=clause['relational'];raw=plan['inputs'].get('raw_nodes')
 if raw is None or 'text' in raw['evidence']['project']:return
 node_columns=['__path','__start','__end',*raw['evidence']['project'],'id','parent','edge_parent','ordinal']
 for stage in plan['stages']:
  if stage['name']=='arguments':
   stage['query']=stage['query'].replace('SELECT n.*,','SELECT '+','.join('n.'+c for c in node_columns)+',',1)
  elif stage['name'] in ('last_methods','last_module_functions'):
   columns=[*node_columns,'name',*(['cls','class_name'] if stage['name']=='last_methods' else [])]
   stage['query']=stage['query'].replace('SELECT *,row_number()','SELECT '+','.join(columns)+',row_number()',1)

def shared_syntax_relations(clause):
 """Use declared tape facts; keep every detector and source gate in SQL/AST."""
 import copy
 plan=clause['relational'];raw=plan['inputs'].get('raw_nodes')
 # Global projected relations may select declarations and selected method bodies
 # across the repository. A file gate cannot represent that row selection.
 if raw is None or plan.get('partitionBy')!='file' or not clause['id'].startswith('quality-contracts'):return False
 batched_reachability(clause)
 return whole_file_syntax_relations(clause)

def whole_file_syntax_relations(clause):
 """Lower a generator-proven whole-file domain through existing syntax facts."""
 import copy
 plan=clause['relational'];raw=plan['inputs']['raw_nodes']
 original=copy.deepcopy(raw)
 def gate_only(rule):
  if not isinstance(rule,dict):return
  parts=rule.get('all')
  if isinstance(parts,list) and P('NODE') in parts:
   captures=[P('NODE'),optional(I(P('PARENT'),'neighbor')),*[optional(F(f,P(f.upper()))) for f in FIELDS]]
   rule['all']=[*[item for item in parts if item not in captures],K('module')];return
  for value in rule.values():
   if isinstance(value,dict):gate_only(value)
   elif isinstance(value,list):
    for item in value:gate_only(item)
 gate_only(original['evidence']['query']['ast']['rule'])
 original['evidence']['project']={'path':'file.path'}
 del plan['inputs']['raw_nodes'];plan['inputs']['syntax_gate']=original
 fields=[f for f in FIELDS if any(name.startswith(f+'_') for name in raw['evidence']['project'])]
 demanded=list(dict.fromkeys(['__path','__start','__end','id','parent','edge_parent','ordinal','text',*raw['evidence']['project'],*[f+'_id' for f in fields]]))
 plan['syntax']={'columns':demanded,'name':'nodes','gate':'syntax_gate','fields':fields,
  'excludeKinds':['comment','string_start','string_content','string_end','escape_sequence','line_continuation'],
  'edges':'edges','ownership':'ownership','owners':{
   'fn':{'kind':'function_definition'},'cls':{'kind':'class_definition'},'top_fn':{'kind':'function_definition','outermost':True}}}
 plan['stages']=[s for s in plan['stages'] if s['name'] not in ('nodes','edges','ownership')]
 # Replace authored identity expressions with the corresponding canonical field ID.
 identity=re.compile(r"(?P<prefix>\b[A-Za-z_]\w*\.)?(?P<field>[a-z_]+)_kind\|\|':'\|\|(?P=prefix)?(?P=field)_start\|\|':'\|\|(?P=prefix)?(?P=field)_end")
 def lower(value):
  if isinstance(value,str):
   value=identity.sub(lambda m:(m['prefix'] or '')+m['field']+'_id',value)
   return re.sub(r'json_quote\(((?:[a-z_]\w*\.)?id)\)',r'json_quote(CAST(\1 AS TEXT))',value)
  if isinstance(value,dict):return {k:lower(v) for k,v in value.items()}
  if isinstance(value,list):return [lower(v) for v in value]
  return value
 columns=[c for c in demanded if c!='text' or 'text' in raw['evidence']['project']]
 for stage in plan['stages']:
  if 'query' in stage:stage['query']=lower(stage['query'])
  if stage['name']=='arguments':stage['query']=stage['query'].replace('SELECT n.*,','SELECT '+','.join('n.'+c for c in columns)+',',1)
  if stage['name'] in ('last_methods','last_module_functions'):
   extra=['name',*(['cls','class_name'] if stage['name']=='last_methods' else [])]
   stage['query']=stage['query'].replace('SELECT *,row_number()','SELECT '+','.join([*columns,*extra])+',row_number()',1)
 plan['query']=lower(plan['query'])
 for stage in plan['stages']:
  if 'reachability' in stage:
   stage['reachability']={'roots':lower(stage['reachability']['roots']),'syntax':'nodes'}
 native_import_identifiers(plan)
 return True

def native_import_identifiers(plan):
 """Collect import subtrees in one separate demand batch, then select identifier keys."""
 target=next((stage for stage in plan['stages'] if stage['name']=='import_identifiers'),None)
 if target is None or any(stage['name']=='import_descendants' for stage in plan['stages']):return
 if 'syntax' not in plan or plan.get('partitionBy')!='file':return
 target['query']="""WITH imported(__path,id) AS MATERIALIZED (
 SELECT __path,node FROM import_descendants)
 SELECT n.__path,n.id FROM nodes n WHERE n.kind='identifier'
 AND n.id IN (SELECT id FROM imported)
 AND (n.__path,n.id) IN (SELECT __path,id FROM imported)"""
 producer={'name':'import_descendants','reachability':{
  'roots':"SELECT __path,id FROM nodes WHERE kind IN ('import_statement','import_from_statement')",
  'syntax':plan['syntax']['name']}}
 stages=[]
 for stage in plan['stages']:
  if stage['name']=='import_ranges':continue
  if stage is target:stages.append(producer)
  stages.append(stage)
 plan['stages']=stages
