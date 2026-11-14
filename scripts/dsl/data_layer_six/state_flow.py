# SPDX-License-Identifier: Apache-2.0
"""Local function reachability and authority/value flow for state tripwires."""
from .common import *
from .state_project import project_inputs,project_stages,PROJECT_QUERY

def flow_clauses(m):
 # One alternation per tail set: one compiled pattern and one call per row instead of one per pair.
 tail=lambda pairs,alias='c':'rx_matches('+alias+'.full_callee,'+q(r'(?:^|\.)(?:'+'|'.join(re.escape(a)+r'(?:\([^)]*\))?\.'+re.escape(b) for a,b in sorted(pairs))+r')$')+')' if pairs else '0'
 cases=[('SPT-003','eager_sql','c.planning AND NOT c.preprocess AND s.origin!=s.fn'),('SPT-011','eager_sql','c.preprocess AND s.origin!=s.fn'),('SPT-019','eager_sql','c.provider_helper AND s.origin!=s.fn'),('SPT-020','ctx_access','(c.planning OR c.ctx_helper) AND s.origin!=s.fn'),('SPT-002','large_column','c.planning AND s.origin!=s.fn'),('SPT-010','resolved_refs','c.planning AND s.origin!=s.fn'),('SPT-005','nested_execute','c.planning AND s.origin!=s.fn'),('SPT-012','materialized_pipeline','c.planning AND s.origin!=s.fn'),('SPT-022','runtime_authority','c.planning'),('SPT-024','runtime_value',"c.planning AND origin_fn.name NOT IN "+literals(m.POSTPROCESS_SCOPE_NAMES)+" AND NOT rx_matches(origin_fn.name,'_post_op_with_refs$')"),('SPT-021','provider_authority','c.planning'),('SPT-026','runtime_callback','c.planning'),('SPT-028','runtime_semantic_projection','c.planning'),('SPT-028','runtime_sequence_projection','NOT c.planning AND c.materializer AND s.origin=s.fn'),('SPT-027','native_python_work',"s.origin=s.fn AND rx_matches(c.__path,'(?:^|/)impl/common/l2070_server_plan/')")]
 allowed=sorted(m.SERVERPLAN_NATIVE_PYTHON_WORK_ALLOWLIST|m.SERVERPLAN_PREPROCESS_RUNTIME_VALUE_ALLOWLIST|m.SERVERPLAN_RUNTIME_CALLBACK_ALLOWLIST)
 # Every clause reads narrow candidate rows; the cross-file builder contracts of
 # scan_project_sources() are part of the flow clause (state_project.py).
 return [narrow_flow_clause(m,tail,cases,allowed),projection_recovery_clause(m)]

def projection_recovery_clause(m):
 """SPT-029: an if/elif in a projection scope that guards on row/alias membership and is followed by recovery.

 The path, scope and condition guards and an else/elif recovery are local syntax on the candidate
 statement. Only two facts join independent rows: a module-level "row"/"rows" constant named in the
 condition, and a recovery call in a later statement of the innermost function's body. Both read
 narrow candidate rows and use span containment; no tree copy.
 """
 entries=['scan_source','scan_tree','scan_project_sources']
 strings=O(K('string'),K('concatenated_string'))
 row_string=A(strings,N(H(K('interpolation'),stop='end')),{'value':{'regex':'^(?:row|rows)$'}})
 within=lambda rule:O(rule,H(rule,stop='end'))
 # File-level facts are utilities, so each is decided once per node (the module) and remembered;
 # their literals are also each input's per-file text prefilter.
 utils={
  'state-projection-recovery-text':A(K('module'),RX('|'.join(re.escape(v) for v in sorted(m.PROJECTION_RECOVERY_NAMES)))),
  'state-projection-scope-text':A(K('module'),RX('|'.join(re.escape(v) for v in sorted(m.PROJECTION_RECOVERY_SCOPE_TOKENS)))),
 }
 texts=A(I({'matches':'state-projection-recovery-text'}),I({'matches':'state-projection-scope-text'}))
 paths={'filePath':{'regex':'|'.join(re.escape(v) for v in sorted(m.PROJECTION_RECOVERY_PATH_TOKENS)),'notRegex':'/tests/'}}
 test=O(A(K('comparison_operator'),RX(r'\b(?:in|is)\b')),call(['isinstance']))
 scope=I(A(O(K('function_definition'),K('class_definition')),F('name',RX('|'.join(re.escape(v) for v in sorted(m.PROJECTION_RECOVERY_SCOPE_TOKENS))))))
 recovery=call(m.PROJECTION_RECOVERY_NAMES)
 utils['state-projection-candidate']=A(O(K('if_statement'),K('elif_clause')),texts,paths,F('condition',within(test)),scope)
 candidate={'matches':'state-projection-candidate'}
 constant=lambda name:A(K('assignment'),I(A(K('expression_statement'),I(K('module'),'neighbor')),'neighbor'),F('left',A(K('identifier'),*([P(name)] if name else []))),F('right',row_string))
 utils['state-projection-row-constants']=A(K('module'),H(constant(None),stop='end'))
 # One input carries every candidate row; its captures say which roles a node plays. A body
 # statement can also be a candidate (an if in a function body) or a nested function.
 cand=A(candidate,P('CAND'),optional(F('condition',within(A(O(names(['row','rows','alias','key','keys']),row_string),P('ATOM'))))),
  optional(H(A(O(K('elif_clause'),K('else_clause')),H(recovery,stop='end'),P('ALT')))))
 fn=A(K('function_definition'),P('FNDEF'))
 body=A(N(K('comment')),I(A(K('block'),I(A(K('function_definition'),P('FN')),'neighbor','body')),'neighbor'),H(recovery,stop='end'))
 named=A(K('identifier'),P('NAME'),O(I(A(candidate,P('IF')),'neighbor','condition'),I(I(A(candidate,P('IF')),'neighbor','condition'))),I({'matches':'state-projection-row-constants'}))
 shapes=O(body,cand,fn,constant('NAME'),named)
 inputs={'facts':E(A(gate(entries),shapes,optional(cand),optional(fn),P('NODE'),texts,paths),
  {'line':'capture.NODE.startLine','cand':'capture.CAND.startByte','atom':'capture.ATOM.startByte','alt':'capture.ALT.startByte',
   'fndef':'capture.FNDEF.startByte','fn':'capture.FN.startByte','name':'capture.NAME','if_start':'capture.IF.startByte','kind':'capture.NODE.kind'})}
 stages=[
  S('candidates',"SELECT DISTINCT __path,__start,__end,line,atom IS NOT NULL AS local_atom,alt IS NOT NULL AS alternative FROM facts WHERE cand IS NOT NULL",('__path','__start')),
  S('functions',"SELECT DISTINCT __path,__start,__end FROM facts WHERE fndef IS NOT NULL",('__path','__start')),
  S('body_statements',"SELECT DISTINCT __path,__start,__end,line,fn FROM facts WHERE fn IS NOT NULL",('__path','fn')),
  S('row_constants',"SELECT DISTINCT __path,name FROM facts WHERE kind='assignment' AND name IS NOT NULL",('__path','name')),
  S('condition_names',"SELECT DISTINCT __path,name,if_start FROM facts WHERE if_start IS NOT NULL",('__path','if_start')),
 ]
 query="""SELECT c.__path AS path,c.__start AS start_byte,c.__end AS end_byte,'SPT-029' AS issue,c.__start AS identity FROM candidates c
 WHERE (c.local_atom OR EXISTS(SELECT 1 FROM condition_names n JOIN row_constants k ON k.__path=n.__path AND k.name=n.name WHERE n.__path=c.__path AND n.if_start=c.__start))
 AND (c.alternative OR EXISTS(SELECT 1 FROM body_statements s WHERE s.__path=c.__path AND s.line>c.line AND s.fn=(SELECT f.__start FROM functions f WHERE f.__path=c.__path AND f.__start<c.__start AND f.__end>=c.__end ORDER BY f.__start DESC LIMIT 1)))"""
 return {'id':'state-projection-recovery','relational':{'inputs':inputs,'stages':stages,'query':query,'key':['issue','identity'],'code':'issue','partitionBy':'file'},
  'violation':{'code':'architecture','message':'Architecture contract violation','anchor':'witness.span'},'_utils':utils}

# Wrapper kinds that never expand as descendant roots (the authored descendants walk).
# Python node kinds that can be a counted ancestor of a flow event: every named kind of the vendored
# tree-sitter-python grammar that has children, less the wrappers the ranking skips (EXPAND_STOP and
# parenthesized_expression) and kinds whose subtrees hold only names and literals (imports, dotted
# names, global/nonlocal, string content).
BRANCH_KINDS=('as_pattern','assert_statement','assignment','attribute','augmented_assignment','await','binary_operator','boolean_operator','call','case_clause','case_pattern','chevron','class_definition','class_pattern','comparison_operator','complex_pattern','concatenated_string','conditional_expression','constrained_type','decorator','default_parameter','delete_statement','dict_pattern','dictionary','dictionary_comprehension','dictionary_splat','dictionary_splat_pattern','elif_clause','else_clause','except_clause','exec_statement','expression_list','finally_clause','for_in_clause','for_statement','format_expression','format_specifier','function_definition','generator_expression','generic_type','if_clause','if_statement','interpolation','keyword_argument','keyword_pattern','lambda','lambda_parameters','list','list_comprehension','list_pattern','list_splat','list_splat_pattern','match_statement','member_type','named_expression','not_operator','pair','parameters','parenthesized_list_splat','pattern_list','print_statement','raise_statement','return_statement','set','set_comprehension','slice','splat_pattern','splat_type','string','subscript','try_statement','tuple','tuple_pattern','type_alias_statement','type_parameter','typed_default_parameter','typed_parameter','unary_operator','union_pattern','union_type','while_statement','with_clause','with_item','with_statement','yield','type_default','as_pattern_target')
EXPAND_STOP=('module','block','expression_statement','argument_list','decorated_definition','type')

def narrow_flow_clause(m,tail,cases,allowed):
 import copy
 """Planning-scope reachability and authority/value flow over narrow candidate rows.

 Every node-local shape (callee names, keyword flags, receiver shapes, positions) is an AST
 condition on its candidate row. The relational part keeps only joins of independently
 matched rows: function ownership, parameter kinds, the taint and semantic-value fixpoints,
 the local call graph and the reachable/transitive sink search. "Body of the outermost
 function" and "descendant of" are span containment; there is no CST copy.
 """
 entries=['scan_source','scan_tree','scan_project_sources']
 # Inside a function; TOP is the outermost enclosing function (the flow context that owns the node).
 inside=I(A(K('function_definition'),N(I(K('function_definition'))),P('TOP')))
 top={'top_start':'capture.TOP.startByte','top_end':'capture.TOP.endByte'}
 # The leading kind test comes first; state.py inserts the file source gates right after it.
 def row(rule,project):
  owned=json.dumps(inside) in json.dumps(rule)
  return E(A(*rule['all'],gate(entries)),{**project,**(top if owned else {})})
 span=lambda name,cap:{name+'_kind':'capture.'+cap+'.kind',name+'_start':'capture.'+cap+'.startByte',name+'_end':'capture.'+cap+'.endByte'}
 arglist_of=lambda cap:I(A(K('argument_list'),I(A(K('call'),P(cap)),'neighbor','arguments')),'neighbor')
 ident=lambda cap:A(K('identifier'),P(cap))
 tail_names=sorted({b for _,b in m.FORBIDDEN_QUERY_CALL_TAILS|m.PREPROCESS_SQL_EXECUTION_CALL_TAILS|m.FORBIDDEN_NATIVE_PYTHON_CALL_CHAINS|m.FORBIDDEN_EXECUTION_TIME_REPLANNING_CALL_TAILS})
 resolved=O(A(K('identifier'),RX(r'^(?:resolved|resolved_refs)$')),A(K('attribute'),F('attribute',A(K('identifier'),RX(r'^(?:resolved|resolved_refs)$')))),A(N(O(K('identifier'),K('attribute'))),RX('resolved')))
 positional=N(O(K('comment'),K('keyword_argument'),K('list_splat'),K('dictionary_splat')))
 strings=O(K('string'),K('concatenated_string'))
 params_of=lambda cap:A(K('parameters'),I(A(K('function_definition'),P(cap)),'neighbor','parameters'))
 # A direct positional argument of a call (generator-expression arguments included).
 arg_of=lambda cap:O(I(A(K('argument_list'),I(A(K('call'),P(cap)),'neighbor','arguments')),'neighbor'),I(A(K('generator_expression'),I(A(K('call'),P(cap)),'neighbor','arguments')),'neighbor'))
 ac={'ac_start':'capture.AC.startByte','ac_end':'capture.AC.endByte'}
 nth=lambda pos,cap:A({'nthChild':{'position':pos,'ofRule':positional}},positional,P(cap),optional(ident(cap+'T')))
 inputs={
  # The per-file gate (state.py appends the module source gates): no other input is read in a
  # file that it does not admit.
  'flow_gate':row(A(K('module'),optional({'$option':'entryPoint','cases':{'scan_project_sources':P('PM')},'default':FALSE})),{'pm':'capture.PM.startByte'}),
  'fns':row(A(K('function_definition'),F('name',P('NAME')),optional(F('return_type',P('RT'))),
    optional(I(A(K('decorated_definition'),RX(r'(?m)^\s*@(?:[\w.]*\.)?pure_lazy\b'),P('DEC')),'neighbor'))),
   {'name':'capture.NAME','rt':'capture.RT','dec':'capture.DEC.startByte'}),
  'classes':row(A(K('class_definition'),F('name',P('NAME'))),{'name':'capture.NAME'}),
  # Identifier rows also say which roles the identifier plays: a parameter name (with its
  # annotation), a direct positional argument, a subscript key, an operand of an `in`
  # comparison, a keyword name. Every input leads with one node kind.
  'idents_in':row(A(K('identifier'),P('ID'),inside,
    optional(O(A(I(params_of('PFN'),'neighbor')),
     I(A(K('typed_parameter'),P('PARAM'),optional(F('type',P('ANN'))),I(params_of('PFN'),'neighbor')),'neighbor'),
     I(A(O(K('default_parameter'),K('typed_default_parameter')),P('PARAM'),optional(F('type',P('ANN'))),I(params_of('PFN'),'neighbor')),'neighbor','name'))),
    optional(arg_of('AC')),
    optional(I(A(K('subscript'),P('KS'),F('value',ident('KR'))),'neighbor','subscript')),
    optional(I(A(K('comparison_operator'),RX(r'\bin\b'),P('CMP')),'neighbor')),
    optional(I(A(K('keyword_argument'),P('KWN')),'neighbor','name'))),
   {'text':'capture.ID','pfn_start':'capture.PFN.startByte','pfn_end':'capture.PFN.endByte','ann':'capture.ANN',**ac,
    'ks_start':'capture.KS.startByte','ks_end':'capture.KS.endByte','kr':'capture.KR','cmp_start':'capture.CMP.startByte','cmp_end':'capture.CMP.endByte','kwn':'capture.KWN.startByte'}),
  'attrs_in':row(A(K('attribute'),inside,F('attribute',P('ATTR')),F('object',P('OBJ')),optional(F('object',ident('OBJT'))),optional(arg_of('AC'))),
   {'attr':'capture.ATTR','objt':'capture.OBJT',**span('obj','OBJ'),**ac}),
  'calls_in':row(A(K('call'),inside,F('function',P('FN')),F('arguments',P('ARGS')),
    optional(F('function',A(K('attribute'),F('attribute',P('ATTR')),F('object',P('RECV')),optional(F('object',ident('RECVT')))))),
    optional(F('function',ident('NAME'))),
    optional(F('function',A(K('attribute'),F('attribute',names(tail_names)),P('FULL')))),
    optional(F('arguments',H(A(K('keyword_argument'),F('name',names(['post_op_with_refs','pre_op_with_refs'])),P('REFS'))))),
    optional(F('arguments',H(A(K('keyword_argument'),F('name',names(['hydrate_large_columns'])),F('value',K('true')),P('HYD'))))),
    optional(F('function',A(K('attribute'),F('object',A(resolved,P('RES')))))),
    # getattr(receiver, key) and <x>.get(key): the first two positional arguments.
    optional(A(F('function',O(names(['getattr']),A(K('attribute'),F('attribute',names(['get']))))),F('arguments',H(nth(1,'A0'))),optional(F('arguments',H(nth(2,'A1')))))),
    optional(arg_of('AC'))),
   {'fkind':'capture.FN.kind','attr':'capture.ATTR','name':'capture.NAME','full':'capture.FULL','recvt':'capture.RECVT',**span('recv','RECV'),
    'refs':'capture.REFS.startByte','hyd':'capture.HYD.startByte','res':'capture.RES.startByte','args_start':'capture.ARGS.startByte','args_end':'capture.ARGS.endByte',
    **span('a0','A0'),'a0t':'capture.A0T',**span('a1','A1'),**ac}),
  'kws_in':row(A(K('keyword_argument'),arglist_of('C'),inside,F('name',P('KN')),F('value',P('V')),optional(F('value',ident('VT')))),
   {'name':'capture.KN','vtext':'capture.VT','c_start':'capture.C.startByte','c_end':'capture.C.endByte',**span('v','V')}),
  'list_splats':row(A(K('list_splat'),arglist_of('C'),inside),{'c_start':'capture.C.startByte','c_end':'capture.C.endByte'}),
  'dict_splats':row(A(K('dictionary_splat'),arglist_of('C'),inside),{'c_start':'capture.C.startByte','c_end':'capture.C.endByte'}),
  # Function-body assignments and module-level constants (constant folding) share one input.
  'assigns_in':row(A(K('assignment'),F('left',P('L')),optional(F('left',ident('LT'))),optional(F('right',P('R'))),
    optional(inside),optional(I(A(K('expression_statement'),P('MODS'),I(K('module'),'neighbor')),'neighbor'))),
   {'l_kind':'capture.L.kind','target_text':'capture.LT',**span('v','R'),'inf':'capture.TOP.startByte','mods':'capture.MODS.startByte'}),
  'subs_in':row(A(K('subscript'),inside,F('value',P('V')),optional(F('value',ident('VT'))),optional(H(A(K('slice'),P('SL')))),
    optional(H(A(strings,P('SKEY'),N(H(K('interpolation'),stop='end')),{'value':{'regex':'^(?:'+'|'.join(re.escape(v) for v in sorted(m.RUNTIME_CALLBACK_AUTHORITY_ATTRS))+')$'}}),'subscript')),
    optional(arg_of('AC'))),
   {**span('v','V'),'vtext':'capture.VT','slice':'capture.SL.startByte','skey_start':'capture.SKEY.startByte','skey_end':'capture.SKEY.endByte',**ac}),
  'comps_in':row(A(K('for_in_clause'),inside,F('right',P('R')),I(A(O(*(K(k) for k in ['list_comprehension','set_comprehension','dictionary_comprehension','generator_expression'])),P('COMP'),H(K('if_clause'))),'neighbor')),
   {**span('r','R'),**span('comp','COMP')}),
  'loops_in':row(A(K('for_statement'),inside,F('right',P('R')),F('left',P('L'))),{**span('r','R'),'l_start':'capture.L.startByte','l_end':'capture.L.endByte'}),
  # Ancestors that the sink ranking counts: every non-wrapper, non-leaf node kind in a function.
  'branches':row(A(O(*(K(k) for k in BRANCH_KINDS)),O(N(K('string')),H(K('interpolation'))),P('B'),inside),{'kind':'capture.B.kind'}),
 }
 # Decoded string literals at the positions string_values joins: a direct call argument or subscript
 # key, or the value of a module-level assignment (constant folding); only authority names matter.
 module_value=I(A(K('assignment'),I(A(K('expression_statement'),I(K('module'),'neighbor')),'neighbor')),'neighbor','right')
 authority_value={'value':{'regex':'^(?:'+'|'.join(re.escape(v) for v in sorted(m.COMPILATION_CONTEXT_AUTHORITY_ATTRS|m.RUNTIME_CALLBACK_AUTHORITY_ATTRS))+')$'}}
 if True:
  inputs['strings_in']=E(A(strings,P('STRING'),N(H(K('interpolation'),stop='end')),O(I(K('argument_list'),'neighbor'),I(K('subscript'),'neighbor'),module_value),authority_value,gate(entries)),
   {'value':'capture.DECODED.text'},transform={'DECODED':{'replace':{'source':'$STRING','value':True,'replace':r'\b\B','by':''}}})

 inputs.update(project_inputs(entries,inside,positional))
 views,inline=merge_roles(inputs,['fns','classes','assigns_in','calls_in','attrs_in','subs_in','kws_in','list_splats','dict_splats','comps_in','loops_in','branches'],entries)
 # The outermost enclosing function, captured by the AST (TOP).
 own=lambda a:f"({a}.top_start*16777216+{a}.top_end)"
 # Node identity: (start, end) packed in one integer; no two candidate rows of different kinds share a span.
 nid=lambda kind,a,s='__start',e='__end':f"({a}.{s}*16777216+{a}.{e})"
 ctx_attrs=literals(m.COMPILATION_CONTEXT_AUTHORITY_ATTRS);rt_attrs=literals(m.RUNTIME_CALLBACK_AUTHORITY_ATTRS)
 auth=q(r'\b(?:'+'|'.join(sorted(m.AUTHORITY_TYPE_NAMES))+r')\b');prov=q(r'\b(?:'+'|'.join(sorted(m.PROVIDER_TYPE_NAMES))+r')\b')
 carrier=lambda a,name:f"EXISTS(SELECT 1 FROM parameter_kinds p WHERE p.__path={a}.__path AND p.fn={a}.fn AND p.name={name} AND p.kind='carrier')"
 stop=literals(EXPAND_STOP)
 # Semantic values: a seed, or a node containing a seed when that node expands as a descendant root.
 sem=lambda a,fn,kind,s,e,node:f"EXISTS(SELECT 1 FROM semantic_seeds v WHERE v.__path={a}.__path AND v.fn={fn} AND v.__start>={s} AND v.__end<={e} AND ({kind} NOT IN {stop} OR v.node={node}))"
 taint=lambda a,node,kind:f"EXISTS(SELECT 1 FROM taints t WHERE t.__path={a}.__path AND t.fn={a}.fn AND t.node={node} AND t.kind={q(kind)})"
 arg_taint=lambda kind:f"(EXISTS(SELECT 1 FROM b_args a JOIN taints t ON t.__path=a.__path AND t.fn=n.fn AND t.node=a.id AND t.kind={q(kind)} WHERE a.__path=n.__path AND a.call=n.id) OR EXISTS(SELECT 1 FROM b_kws k JOIN taints t ON t.__path=k.__path AND t.fn=n.fn AND t.node=k.value AND t.kind={q(kind)} WHERE k.__path=n.__path AND k.call=n.id))"
 pure=literals(m.PURE_SCOPE_NAMES)
 # Inside a call's arguments node (so inside one of its arguments, positional or keyword) and not in a splat.
 within_args=lambda v,c:f"{v}.__start>={c}.args_start AND {v}.__end<={c}.args_end AND NOT ({v}.__start={c}.args_start AND {v}.__end={c}.args_end) AND NOT EXISTS(SELECT 1 FROM splats sp WHERE sp.__path={c}.__path AND sp.call={c}.id AND {v}.__start>=sp.__start AND {v}.__end<=sp.__end)"
 has_param=lambda cond:f"EXISTS(SELECT 1 FROM b_params p WHERE p.__path=f.__path AND p.fn=(f.__start*16777216+f.__end) AND {cond})"
 stages=[*views,
 S('project_mode',"SELECT __path FROM flow_gate WHERE pm IS NOT NULL"),
 S('b_params',"SELECT __path,__start,(pfn_start*16777216+pfn_end) AS fn,text AS name,ann,row_number() OVER(PARTITION BY __path,pfn_start,pfn_end ORDER BY __start)-1 AS position FROM idents_in WHERE pfn_start IS NOT NULL",('__path','fn','name')),
 S('contexts',"SELECT f.__path,(f.__start*16777216+f.__end) AS id,f.__start,f.__end,f.name,f.rt,"
  "coalesce((SELECT group_concat(k.name,'.') FROM (SELECT k.name FROM classes k WHERE k.__path=f.__path AND k.__start<f.__start AND k.__end>=f.__end ORDER BY k.__start) k),'') AS class_name,"
  "rx_matches(f.name,'_lazy$') OR f.name IN "+pure+" OR f.name='pending_pipeline_effects' OR rx_matches(f.name,'^apply_.*_effect$') OR f.dec IS NOT NULL AS planning,"
  "f.name='preprocess' AS preprocess,"
  "substr(f.name,-11)='sql_builder' AND f.name NOT IN "+pure+" AND "+has_param("rx_matches(coalesce(p.ann,''),'\\bCompilationContext\\b')")+" AS ctx_helper,"
  "f.__path="+q(m.BACKEND_PROVIDER_PURITY_FILE)+" AND rx_matches(f.name,"+q('|'.join(sorted(m.BACKEND_PROVIDER_PURITY_NAME_TOKENS)))+") AS provider_helper,"
  +has_param("p.name='runtime_refs'")+" AND (rx_matches(f.name,'^materialize_.*builder$') OR "+has_param("rx_matches(coalesce(p.ann,''),'\\bRuntimeBuilderSpec\\b')")+") AS materializer "
  "FROM fns f WHERE NOT EXISTS(SELECT 1 FROM fns o WHERE o.__path=f.__path AND o.__start<f.__start AND o.__end>=f.__end)",('__path','id'),('__path','name'),('__path','__start')),
 # Identifier occurrences matter only by name: parameter names, assignment targets (module
 # constants included) and loop targets. Other identifiers never join.
 S('interest_names',"SELECT __path,name FROM b_params UNION SELECT __path,target_text FROM assigns_in WHERE target_text IS NOT NULL UNION SELECT i.__path,i.text FROM loops_in l JOIN idents_in i ON i.__path=l.__path AND i.__start>=l.l_start AND i.__end<=l.l_end",('__path','name')),
 S('b_idents',"SELECT __path,__start,__end,"+nid("'identifier'",'i')+" AS id,text,"+own('i')+" AS fn,CASE WHEN i.ac_start IS NOT NULL THEN (i.ac_start*16777216+i.ac_end) END AS call,ks_start,ks_end,kr,cmp_start,cmp_end,kwn FROM idents_in i WHERE EXISTS(SELECT 1 FROM interest_names x WHERE x.__path=i.__path AND x.name=i.text)",('__path','fn','text'),('__path','__start'),('__path','id')),
 S('b_attrs',"SELECT __path,__start,__end,"+nid("'attribute'",'a')+" AS id,attr,objt,"+nid('a.obj_kind','a','obj_start','obj_end')+" AS object,"+own('a')+" AS fn,CASE WHEN a.ac_start IS NOT NULL THEN (a.ac_start*16777216+a.ac_end) END AS call FROM attrs_in a",('__path','id')),
 S('b_calls',"SELECT __path,__start,__end,"+nid("'call'",'c')+" AS id,coalesce(attr,name) AS callee,fkind AS callee_kind,coalesce(full,name) AS full_callee,"
  +nid('c.recv_kind','c','recv_start','recv_end')+" AS receiver,recv_kind,recv_start,recv_end,recvt,refs IS NOT NULL AS refs,hyd IS NOT NULL AS hyd,res IS NOT NULL AS res,full,args_start,args_end,a0_kind,a0_start,a0_end,a0t,a1_kind,a1_start,a1_end,"+own('c')+" AS fn,CASE WHEN c.ac_start IS NOT NULL THEN (c.ac_start*16777216+c.ac_end) END AS call FROM calls_in c",('__path','id'),('__path','callee'),('__path','__start'),('__path','receiver')),
 S('b_kws',"SELECT __path,__start,__end,(c_start*16777216+c_end) AS call,name,"+nid('k.v_kind','k','v_start','v_end')+" AS value,v_kind,v_start,v_end,vtext FROM kws_in k",('__path','call','name'),('__path','value')),
 S('b_assigns',"SELECT __path,__start,__end,"+nid("'assignment'",'a')+" AS id,l_kind AS target_kind,target_text,"+nid('a.v_kind','a','v_start','v_end')+" AS value,v_kind,v_start,v_end,"+own('a')+" AS fn FROM assigns_in a WHERE inf IS NOT NULL",('__path','fn','v_start'),('__path','value')),
 S('b_subs',"SELECT __path,__start,__end,"+nid("'subscript'",'s')+" AS id,"+nid('s.v_kind','s','v_start','v_end')+" AS value,v_kind,v_start,v_end,vtext,slice IS NOT NULL AS slice,skey_start,skey_end,"+own('s')+" AS fn,CASE WHEN s.ac_start IS NOT NULL THEN (s.ac_start*16777216+s.ac_end) END AS call FROM subs_in s",('__path','id'),('__path','value')),
 S('b_args',"SELECT __path,id,call FROM b_idents WHERE call IS NOT NULL UNION ALL SELECT __path,id,call FROM b_attrs WHERE call IS NOT NULL UNION ALL SELECT __path,id,call FROM b_calls WHERE call IS NOT NULL UNION ALL SELECT __path,id,call FROM b_subs WHERE call IS NOT NULL",('__path','call'),('__path','id')),
 S('splats',"SELECT __path,__start,__end,(c_start*16777216+c_end) AS call FROM list_splats UNION ALL SELECT __path,__start,__end,(c_start*16777216+c_end) FROM dict_splats",('__path','call')),
 S('callback_parameters',"SELECT DISTINCT c.__path,c.id AS fn,p.name,k.name AS callback_kind,p.position FROM contexts c JOIN b_calls call ON call.__path=c.__path AND call.fn=c.id AND call.callee='CompiledStep' JOIN b_kws k ON k.__path=call.__path AND k.call=call.id AND k.name IN ('post_op','post_op_with_refs') AND k.v_kind='identifier' JOIN fns nested ON nested.__path=c.__path AND nested.name=k.vtext AND nested.__start>c.__start AND nested.__end<=c.__end JOIN b_params p ON p.__path=nested.__path AND p.fn=(nested.__start*16777216+nested.__end) WHERE p.position IN (1,2)",('__path','fn','name')),
 S('parameter_kinds',"SELECT p.__path,p.fn,p.name,'authority' AS kind FROM b_params p WHERE p.name IN ('txn','read_txn','write_txn','conn','connection','sql_conn') OR rx_matches(coalesce(p.ann,''),"+auth+") "
  "UNION SELECT p.__path,p.fn,p.name,'provider' FROM b_params p WHERE rx_matches(coalesce(p.ann,''),"+prov+") "
  "UNION SELECT p.__path,p.fn,p.name,'runtime_value' FROM b_params p LEFT JOIN contexts c ON c.__path=p.__path AND c.id=p.fn WHERE p.name='resolved' OR rx_matches(coalesce(p.ann,''),'\\bResolvedRefs\\b') OR c.materializer AND p.name='runtime_refs' "
  "UNION SELECT p.__path,p.fn,p.name,'carrier' FROM b_params p WHERE p.name IN ('hydrator','runtime_hydrator','runtime_refs') OR rx_matches(coalesce(p.ann,''),'\\bHydrator\\b') "
  "UNION SELECT __path,fn,name,CASE WHEN callback_kind='post_op_with_refs' AND position=2 THEN 'runtime_value' ELSE 'carrier' END FROM callback_parameters WHERE position=1 OR callback_kind='post_op_with_refs' AND position=2 "
  "UNION SELECT __path,fn,name,'carrier' FROM callback_parameters WHERE callback_kind='post_op_with_refs' AND position=2",('__path','fn','name','kind')),
 # Literal keys, and identifiers that name a module-level string constant (constant folding).
 S('string_values',"SELECT __path,__start,__end,value FROM strings_in "
  "UNION SELECT i.__path,i.__start,i.__end,s.value FROM assigns_in k JOIN strings_in s ON s.__path=k.__path AND s.__start=k.v_start AND s.__end=k.v_end JOIN b_idents i ON i.__path=k.__path AND i.text=k.target_text WHERE k.mods IS NOT NULL AND k.v_kind IN ('string','concatenated_string')",('__path','__start','__end')),
 S('special_attributes',"SELECT a.__path,a.fn,a.id AS node,'attribute' AS nkind,a.__start,a.__end,CASE WHEN a.objt='ctx' AND a.attr IN "+ctx_attrs+" THEN 'context' ELSE 'carrier' END AS owner,a.attr FROM b_attrs a WHERE a.objt='ctx' AND a.attr IN "+ctx_attrs+" OR a.attr IN "+rt_attrs+" AND "+carrier('a','a.objt')+
  " UNION SELECT c.__path,c.fn,c.id,'call',c.__start,c.__end,CASE WHEN c.a0t='ctx' THEN 'context' ELSE 'carrier' END,s.value FROM b_calls c JOIN string_values s ON s.__path=c.__path AND s.__start=c.a1_start AND s.__end=c.a1_end WHERE c.full_callee='getattr' AND c.a0_kind='identifier' AND (c.a0t='ctx' AND s.value IN "+ctx_attrs+" OR s.value IN "+rt_attrs+" AND "+carrier('c','c.a0t')+")"
  " UNION SELECT c.__path,c.fn,c.id,'call',c.__start,c.__end,'carrier',s.value FROM b_calls c JOIN string_values s ON s.__path=c.__path AND s.__start=c.a0_start AND s.__end=c.a0_end WHERE c.callee='get' AND c.callee_kind='attribute' AND s.value IN "+rt_attrs+" AND "+carrier('c','c.recvt')+
  " UNION SELECT k.__path,k.fn,(k.__start*16777216+k.__end),'subscript',k.__start,k.__end,'carrier',s.value FROM b_subs k JOIN string_values s ON s.__path=k.__path AND s.__start=k.skey_start AND s.__end=k.skey_end WHERE k.v_kind='identifier' AND "+carrier('k','k.vtext')+
  " UNION SELECT k.__path,k.fn,(k.ks_start*16777216+k.ks_end),'subscript',k.ks_start,k.ks_end,'carrier',s.value FROM b_idents k JOIN string_values s ON s.__path=k.__path AND s.__start=k.__start AND s.__end=k.__end WHERE k.ks_start IS NOT NULL AND s.value IN "+rt_attrs+" AND "+carrier('k','k.kr'),('__path','fn','node')),
 S('return_kinds',"WITH RECURSIVE runtime(__path,name) AS (SELECT DISTINCT c.__path,c.name FROM contexts c JOIN special_attributes a ON a.__path=c.__path AND a.fn=c.id AND a.owner='carrier' WHERE EXISTS(SELECT 1 FROM parameter_kinds p WHERE p.__path=c.__path AND p.fn=c.id AND p.kind='carrier') "
  "UNION SELECT DISTINCT c.__path,c.name FROM contexts c JOIN b_calls call ON call.__path=c.__path AND call.fn=c.id AND call.callee_kind='identifier' JOIN runtime r ON r.__path=call.__path AND r.name=call.callee WHERE EXISTS(SELECT 1 FROM b_idents n JOIN parameter_kinds p ON p.__path=c.__path AND p.fn=c.id AND p.name=n.text AND p.kind='carrier' WHERE n.__path=call.__path AND "+within_args('n','call')+" AND NOT EXISTS(SELECT 1 FROM b_kws k WHERE k.__path=call.__path AND k.call=call.id AND n.__start>=k.__start AND n.__end<=k.__end))) "
  "SELECT __path,name,'runtime_authority' AS kind FROM runtime UNION SELECT c.__path,c.name,'authority' FROM contexts c WHERE rx_matches(coalesce(c.rt,''),"+auth+") UNION SELECT c.__path,c.name,'provider' FROM contexts c WHERE rx_matches(coalesce(c.rt,''),"+prov+")",('__path','name','kind')),
 S('expression_edges',"SELECT __path,fn,id AS dst,__start,__end,object AS src FROM b_attrs UNION SELECT __path,fn,id,__start,__end,receiver FROM b_calls WHERE receiver IS NOT NULL UNION SELECT __path,fn,id,__start,__end,value FROM b_subs",('__path','src')),
 S('taints',"WITH RECURSIVE taint(__path,fn,node,__start,__end,kind) AS (SELECT n.__path,n.fn,n.id,n.__start,n.__end,p.kind FROM b_idents n JOIN parameter_kinds p ON p.__path=n.__path AND p.fn=n.fn AND p.name=n.text WHERE p.kind!='carrier' "
  "UNION SELECT __path,fn,node,__start,__end,'authority' FROM special_attributes WHERE owner='carrier' OR attr IN ('txn','sql_connection') "
  "UNION SELECT c.__path,c.fn,c.id,c.__start,c.__end,r.kind FROM b_calls c JOIN return_kinds r ON (r.__path=c.__path OR EXISTS(SELECT 1 FROM project_mode)) AND r.name=c.callee "
  "UNION SELECT t.__path,t.fn,e.dst,e.__start,e.__end,t.kind FROM taint t JOIN expression_edges e ON e.__path=t.__path AND e.src=t.node AND e.fn=t.fn "
  "UNION SELECT t.__path,t.fn,n.id,n.__start,n.__end,t.kind FROM taint t JOIN b_assigns a ON a.__path=t.__path AND a.value=t.node AND a.fn=t.fn JOIN b_idents n ON n.__path=a.__path AND n.fn=t.fn AND n.text=a.target_text WHERE a.target_kind='identifier' AND t.kind IN ('authority','provider','runtime_value')) SELECT * FROM taint",('__path','fn','node','kind')),
 S('semantic_seeds',"WITH RECURSIVE semantic(__path,fn,node,__start,__end) AS (SELECT __path,fn,node,__start,__end FROM taints WHERE kind='runtime_value' "
  "UNION SELECT n.__path,n.fn,n.id,n.__start,n.__end FROM b_idents n JOIN parameter_kinds p ON p.__path=n.__path AND p.fn=n.fn AND p.name=n.text AND p.kind='carrier' "
  "UNION SELECT n.__path,n.fn,n.id,n.__start,n.__end FROM semantic s JOIN b_assigns a ON a.__path=s.__path AND a.fn=s.fn AND a.v_start<=s.__start AND a.v_end>=s.__end AND (a.v_kind NOT IN "+stop+" OR a.value=s.node) JOIN b_idents n ON n.__path=a.__path AND n.fn=a.fn AND n.text=a.target_text WHERE a.target_kind='identifier') SELECT * FROM semantic",('__path','fn','__start')),
 ]
 sinks=[]
 def sink(kind,table,where,node='n.id',priority='1',start='n.__start',end='n.__end'):
  sinks.append(f"SELECT n.__path,n.fn,{node} AS node,{q(kind)} AS kind,{priority} AS priority,{start} AS __start,{end} AS __end FROM {table} n WHERE {where}")
 tailed=lambda pairs:"n.full IS NOT NULL AND "+tail(pairs,'n')
 # Taint-driven events start from the (few) tainted nodes and join the candidate rows they touch.
 def tsink(kind,taint_kind,table,on,where='1',node='n.id',priority='1',start='n.__start',end='n.__end',via=''):
  sinks.append(f"SELECT n.__path,n.fn,{node} AS node,{q(kind)} AS kind,{priority} AS priority,{start} AS __start,{end} AS __end FROM taints t {via} JOIN {table} n ON n.__path=t.__path AND n.fn=t.fn AND {on} WHERE t.kind={q(taint_kind)} AND {where}")
 sink('eager_sql','b_calls',"n.callee IN "+literals(m.DIRECT_IO_NAMES)+" OR n.callee_kind='identifier' AND n.callee IN "+literals(m.PREPROCESS_SQL_EXECUTION_NAMES)+" OR "+tailed(m.FORBIDDEN_QUERY_CALL_TAILS|m.PREPROCESS_SQL_EXECUTION_CALL_TAILS))
 tsink('eager_sql','authority','b_calls','n.receiver=t.node',"n.callee IN "+literals(m.PREPROCESS_SQL_EXECUTION_NAMES))
 sink('native_python_work','b_calls',"n.callee IN "+literals(m.FORBIDDEN_NATIVE_PYTHON_CALL_NAMES)+" OR "+tailed(m.FORBIDDEN_NATIVE_PYTHON_CALL_CHAINS))
 sink('runtime_callback','b_calls',"n.callee='CompiledStep' AND n.refs")
 sink('large_column','b_calls',"n.callee IN "+literals(m.LARGE_COLUMN_HELPER_NAMES|m.FORBIDDEN_TERMINAL_HYDRATION_HELPER_NAMES)+" OR n.callee='convert_row_to_model' AND n.hyd")
 sink('resolved_refs','b_calls',"n.callee IN "+literals(m.FORBIDDEN_RESOLVED_RUNTIME_ACCESS_METHODS)+" AND n.res")
 sink('nested_execute','b_calls',"n.callee IN "+literals(m.FORBIDDEN_EXECUTION_TIME_REPLANNING_NAMES)+" OR "+tailed(m.FORBIDDEN_EXECUTION_TIME_REPLANNING_CALL_TAILS))
 sink('materialized_pipeline','b_attrs',"n.attr='materialized_pipeline'")
 sink('materialized_pipeline','special_attributes',"n.attr='materialized_pipeline' AND n.owner='context'",node='n.node')
 sink('ctx_access','special_attributes',"n.owner='context'",node='n.node')
 sink('runtime_authority','special_attributes',"n.owner='carrier' AND n.nkind IN ('attribute','call')",node='n.node')
 sink('runtime_authority','b_calls',"n.callee IN "+literals(m.RUNTIME_CALLBACK_AUTHORITY_HELPERS))
 tsink('runtime_authority','runtime_authority','b_calls','n.id=t.node')
 for kind in ('runtime_authority','runtime_value'):
  tsink(kind,kind,'b_assigns','n.value=t.node',priority='0')
 tsink('runtime_value','runtime_value','b_calls','n.id=t.node')
 tsink('runtime_value','runtime_value','b_calls','n.receiver=t.node')
 tsink('runtime_value','runtime_value','b_calls','n.id=a.call',via='JOIN b_args a ON a.__path=t.__path AND a.id=t.node')
 tsink('runtime_value','runtime_value','b_calls','n.id=k.call',via='JOIN b_kws k ON k.__path=t.__path AND k.value=t.node')
 tsink('runtime_value','runtime_value','b_subs','n.value=t.node',"n.v_kind='identifier'")
 tsink('runtime_value','runtime_value','b_idents','n.id=t.node',"n.cmp_start IS NOT NULL",node=nid("'comparison_operator'",'n','cmp_start','cmp_end'),start='n.cmp_start',end='n.cmp_end')
 tsink('provider_authority','provider','b_calls','n.receiver=t.node',arg_taint('authority'))
 sink('runtime_semantic_projection','b_calls',"n.callee IN "+literals(m.RUNTIME_SEMANTIC_PROJECTION_CALL_NAMES)+" AND EXISTS(SELECT 1 FROM semantic_seeds v WHERE v.__path=n.__path AND v.fn=n.fn AND "+within_args('v','n')+" AND NOT EXISTS(SELECT 1 FROM b_idents w WHERE w.__path=v.__path AND w.id=v.node AND w.kwn IS NOT NULL)) OR n.callee IN "+literals(m.RUNTIME_SEMANTIC_PROJECTION_METHOD_NAMES)+" AND n.receiver IS NOT NULL AND "+sem('n','n.fn','n.recv_kind','n.recv_start','n.recv_end','n.receiver'))
 sink('runtime_semantic_projection','b_subs',"n.slice AND "+sem('n','n.fn','n.v_kind','n.v_start','n.v_end','n.value'))
 sink('runtime_semantic_projection',"(SELECT *,"+own('c')+" AS fn FROM comps_in c)",sem('n','n.fn','n.r_kind','n.r_start','n.r_end',nid('n.r_kind','n','r_start','r_end')),node=nid('n.comp_kind','n','comp_start','comp_end'),start='n.comp_start',end='n.comp_end')
 projections="(SELECT __path,__start,__end FROM b_subs UNION ALL SELECT __path,__start,__end FROM b_calls WHERE callee IN ('get','getattr'))"
 loop=(sem('n','n.fn','n.r_kind','n.r_start','n.r_end',nid('n.r_kind','n','r_start','r_end'))+" AND EXISTS(SELECT 1 FROM b_calls c WHERE c.__path=n.__path AND c.__start>=n.__start AND c.__end<=n.__end AND c.callee IN ('append','extend'))"
  " AND EXISTS(SELECT 1 FROM "+projections+" pr WHERE pr.__path=n.__path AND pr.__start>=n.__start AND pr.__end<=n.__end AND EXISTS(SELECT 1 FROM b_idents u JOIN b_idents t ON t.__path=u.__path AND t.text=u.text AND t.__start>=n.l_start AND t.__end<=n.l_end WHERE u.__path=pr.__path AND u.__start>=pr.__start AND u.__end<=pr.__end))")
 for kind in ('runtime_semantic_projection','runtime_sequence_projection'):
  sink(kind,"(SELECT *,"+own('l')+" AS fn FROM loops_in l)",loop,node=nid("'for_statement'",'n'))
 stages.extend([
 S('direct_events',' UNION '.join(sinks),('__path','fn','kind')),
 # Rank the shallowest event: count the non-wrapper ancestors inside the function (span
 # containment over branch rows); only groups with a choice need the count.
 S('direct_sinks',"SELECT __path,fn,node,kind,__start,__end FROM (SELECT d.*,row_number() OVER(PARTITION BY d.__path,d.fn,d.kind ORDER BY d.priority,d.depth,d.__start) AS chosen FROM "
  "(SELECT e.*,CASE WHEN count(*) OVER(PARTITION BY e.__path,e.fn,e.kind)>1 THEN (SELECT count(*) FROM nodes_in b WHERE b.__path=e.__path AND b.__start>=c.__start AND b.__start<=e.__start AND b.__end>=e.__end) END AS depth FROM direct_events e JOIN contexts c ON c.__path=e.__path AND c.id=e.fn) d) WHERE chosen=1",('__path','fn','kind')),
 S('call_graph',"SELECT DISTINCT c.__path,c.fn,c.id AS node,c.__start,c.__end,target.id AS target FROM b_calls c JOIN contexts owner ON owner.__path=c.__path AND owner.id=c.fn JOIN contexts target ON target.__path=c.__path AND target.name=c.callee AND target.id!=c.fn WHERE c.callee_kind='identifier' AND target.class_name='' OR c.callee_kind='attribute' AND target.class_name=owner.class_name AND owner.class_name!='' AND c.recvt IN ('self','cls')",('__path','fn','target'),('__path','target')),
 S('reachable',"WITH RECURSIVE reachable(__path,fn) AS (SELECT c.__path,c.id FROM contexts c WHERE c.planning OR c.preprocess OR c.materializer OR c.ctx_helper OR c.provider_helper OR rx_matches(c.__path,'(?:^|/)impl/common/l2070_server_plan/') AND EXISTS(SELECT 1 FROM direct_sinks s WHERE s.__path=c.__path AND s.fn=c.id AND s.kind='native_python_work') UNION SELECT r.__path,g.target FROM reachable r JOIN call_graph g ON g.__path=r.__path AND g.fn=r.fn) SELECT * FROM reachable",('__path','fn')),
 S('transitive_sinks',"WITH RECURSIVE sink(__path,fn,node,__start,__end,kind,origin,route) AS (SELECT s.__path,s.fn,s.node,s.__start,s.__end,s.kind,s.fn,'|'||s.fn||'|' FROM direct_sinks s JOIN reachable r ON r.__path=s.__path AND r.fn=s.fn UNION ALL SELECT g.__path,g.fn,g.node,g.__start,g.__end,s.kind,s.origin,s.route||g.fn||'|' FROM sink s JOIN call_graph g ON g.__path=s.__path AND g.target=s.fn JOIN reachable r ON r.__path=g.__path AND r.fn=g.fn WHERE instr(s.route,'|'||g.fn||'|')=0 AND NOT EXISTS(SELECT 1 FROM direct_sinks d WHERE d.__path=g.__path AND d.fn=g.fn AND d.kind=s.kind)) "
  "SELECT __path,fn,node,__start,__end,kind,origin FROM (SELECT s.*,row_number() OVER(PARTITION BY s.__path,s.fn,s.kind ORDER BY length(s.route),s.__start) AS chosen FROM sink s) WHERE chosen=1",('__path','fn','kind')),
 S('signed_exceptions',"SELECT value AS key FROM json_each("+q(json.dumps(allowed,separators=(',',':')))+")",('key',)),
 *project_stages(own),
 ])
 role_ref=re.compile(r'\b(FROM|JOIN)\s+('+'|'.join(inline)+r')\b')
 for stage in stages:
  if isinstance(stage['query'],str):stage['query']=role_ref.sub(lambda x:x[1]+' ('+inline[x[2]]+')',stage['query'])
 for stage in stages:
  if stage['name']=='return_kinds':
   stage['query']={'$option':'returnKindsHex','escape':'regex','template':stage['query']+" UNION SELECT DISTINCT c.__path,entry.key,kind.value FROM contexts c,json_each(CAST(X'{}' AS TEXT)) entry,json_each(entry.value) kind"}
 # One SELECT over every (code, sink kind) case; each case keeps its own scope condition.
 mapping=' UNION ALL '.join(f"SELECT {i} AS i,{q(code)} AS code,{q(kind)} AS kind" for i,(code,kind,_) in enumerate(cases))
 condition='CASE m.i '+' '.join(f'WHEN {i} THEN ({where})' for i,(_,_,where) in enumerate(cases))+' END'
 flow=("SELECT s.__path AS path,s.__start AS start_byte,s.__end AS end_byte,m.code AS issue,s.fn||':'||s.kind AS identity,'flow' AS detector FROM transitive_sinks s JOIN ("+mapping+") m ON m.kind=s.kind "
  "JOIN contexts c ON c.__path=s.__path AND c.id=s.fn JOIN contexts origin_fn ON origin_fn.__path=s.__path AND origin_fn.id=s.origin WHERE "+condition)
 full=[flow];production=[flow+" AND NOT EXISTS(SELECT 1 FROM signed_exceptions exception WHERE exception.key=m.code||'::'||CASE WHEN c.class_name='' THEN c.name ELSE c.class_name||'.'||c.name END||'::'||c.__path)"]
 def direct(code,where,join=''):
  sql=f"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,{q(code)} AS issue,n.id AS identity,'direct-flow' AS detector FROM b_calls n {join} WHERE {where}"
  full.append(sql);production.append(sql)
 special="JOIN special_attributes s ON s.__path=n.__path AND s.node=n.id JOIN contexts c ON c.__path=s.__path AND c.id=s.fn"
 direct('SPT-020',"NOT rx_matches(n.__path,'/tests/') AND s.owner='context' AND s.attr IN ('txn','sql_connection') AND (c.planning OR c.ctx_helper)",special)
 direct('SPT-012',"s.owner='context' AND s.attr='materialized_pipeline' AND c.planning",special)
 direct('SPT-023',"n.callee='convert_row_to_model' AND NOT rx_matches(n.__path,'/tests/') AND EXISTS(SELECT 1 FROM fns nested JOIN callback_parameters p ON p.__path=nested.__path AND p.name IN (SELECT a.name FROM b_params a WHERE a.__path=nested.__path AND a.fn=(nested.__start*16777216+nested.__end)) JOIN contexts outer_fn ON outer_fn.__path=p.__path AND outer_fn.id=p.fn WHERE nested.__path=n.__path AND nested.__start<=n.__start AND nested.__end>=n.__end AND nested.__start>outer_fn.__start AND nested.__end<=outer_fn.__end)")
 full=' UNION ALL '.join(full);production=' UNION ALL '.join(production)
 query={'$option':'entryPoint','cases':{'all':choice('oracleAnalyzer',full,production),**{e:full for e in entries},'scan_project_sources':full+' UNION ALL '+PROJECT_QUERY},'default':'SELECT * FROM ('+full+') WHERE 0'}
 return {'id':'state-flow-narrow','relational':{'inputs':inputs,'stages':stages,'query':query,'key':['detector','issue','identity'],'code':'issue',
  'partitionBy':{'$option':'entryPoint','cases':{'scan_project_sources':'repository'},'default':'file'},'requireNonemptyInput':'flow_gate'},
  'violation':{'code':'architecture','message':'Architecture contract violation','anchor':'witness.span'}}


def merge_roles(inputs,roles,entries,materialize=('fns','classes')):
 """Fold kind-headed role inputs into one input (each input costs a corpus pass of its own).

 Each role keeps its rule as one alternative, tagged by a role capture; its captures and columns
 are prefixed. A view stage per role restores the role's own table and column names. The last
 role (branches) takes every remaining row, so the merged table also holds every counted ancestor.
 """
 import copy
 def rename(value,prefix):
  if isinstance(value,dict):
   return {k:('$'+prefix+v[1:] if k=='pattern' and isinstance(v,str) and v.startswith('$') and v!='$_' else rename(v,prefix)) for k,v in value.items()}
  if isinstance(value,list):return [rename(v,prefix) for v in value]
  return value
 kinds=[];alternatives=[];project={'kind':'capture.NODE.kind'};views=[];inline={}
 for index,role in enumerate(roles):
  prefix='R%d_'%index
  relation=inputs.pop(role)['evidence']
  rule=relation['query']['ast']['rule']['all'][:-1]  # less the trailing entry gate
  head=rule[0]
  kinds+=[head] if 'kind' in head else head['any']
  alternatives.append(A(P(prefix+'ROLE'),*rename(rule,prefix)))
  columns={}
  for name,source in relation['project'].items():
   column='r%d_%s'%(index,name)
   project[column]=source.replace('capture.','capture.'+prefix,1);columns[name]=column
  project['r%d_role'%index]='capture.'+prefix+'ROLE.startByte'
  select=','.join(f'{c} AS {n}' for n,c in columns.items())
  where='' if role=='branches' else f" WHERE r{index}_role IS NOT NULL"
  sql='SELECT __path,__start,__end'+(','+select if select else '')+' FROM nodes_in'+where
  # Small tables that correlated subqueries probe, and the indexed ancestor rows, are
  # materialized; every other role is read once, inline.
  if role in materialize:views.append(S(role,sql,('__path','__start')))
  elif role=='branches':pass
  else:inline[role]=sql
 head=[];[head.append(k) for k in kinds if k not in head]
 inputs['nodes_in']=E(A(O(*head),P('NODE'),O(*alternatives),gate(entries)),project)
 return views,inline
