# SPDX-License-Identifier: Apache-2.0
"""State tripwire syntax, planning scopes, and source-level event identities."""
from .common import *

def state_clauses(m):
 clauses=[];utils={}
 strings=O(K('string'),K('concatenated_string'))
 literal=lambda values:A(strings,{'value':{'regex':'^(?:'+'|'.join(re.escape(v) for v in sorted(values))+')$'}})
 args=lambda rule:F('arguments',H(rule))
 arg=lambda pos,rule:args(A(rule,{'nthChild':{'position':pos,'ofRule':N(K('comment'))}}))
 kw=lambda name,rule:A(K('keyword_argument'),F('name',names([name])),F('value',rule))
 nottest=N({'filePath':{'regex':r'/tests/'}})
 pending=O(names(['pending_pipeline_effects']),A(K('identifier'),RX(r'^apply_.*_effect$')))
 pure_fn=O(A(K('function_definition'),F('name',O(names(m.PURE_SCOPE_NAMES),A(K('identifier'),RX(r'_lazy$')),pending))),
  A(K('function_definition'),I(A(K('decorated_definition'),H(A(K('decorator'),H(term(['pure_lazy']))))),'neighbor')))
 pure=I(pure_fn)
 preprocess=I(A(O(K('function_definition'),K('class_definition')),F('name',names(['preprocess']))))
 annword=lambda values:O(names(values),H(names(values),stop='end'))
 typedparam=A(O(K('typed_parameter'),K('typed_default_parameter')),F('type',annword(['CompilationContext'])))
 sql_builder_fn=A(K('function_definition'),F('name',A(K('identifier'),RX('sql_builder$'),N(names(m.PURE_SCOPE_NAMES)))),F('parameters',H(typedparam)))
 symbolic=O(pure,I(sql_builder_fn))
 provider=I(A(K('function_definition'),{'filePath':{'regex':'^'+re.escape(m.BACKEND_PROVIDER_PURITY_FILE)+'$'}},F('name',RX('|'.join(re.escape(v) for v in sorted(m.BACKEND_PROVIDER_PURITY_NAME_TOKENS))))))
 def tail_rule(tails):
  items=[]
  for first,last in sorted(tails):
   key='state-tail-'+first
   utils[key]=O(term([first]),A(K('call'),F('function',{'matches':key})),A(K('parenthesized_expression'),H({'matches':key})))
   items.append(A(K('call'),F('function',A(K('attribute'),F('attribute',names([last])),F('object',{'matches':key})))))
  return O(*items)
 def event(id,rule,code,detail=None,each=None,anchor=None):
  root=A(gate(['scan_source','scan_tree','scan_project_sources']),rule,P('ROOT'),
   optional(I(A(K('function_definition'),P('SCOPE')),K('function_definition'))),
   optional(O(A(K('call'),F('function',O(A(K('identifier'),P('TERM')),A(K('attribute'),F('attribute',P('TERM')))))),
    A(O(K('class_definition'),K('function_definition')),F('name',P('TERM'))),A(K('attribute'),F('attribute',P('TERM'))))))
  project={'line':'capture.ROOT.startLine','scope':'capture.SCOPE.startByte','terminal':'capture.TERM'}
  if detail:project['detail']=detail
  input_=E(root,project,each=each,anchor=anchor)
  group=['line','scope','terminal',*(['detail'] if detail else [])]
  query='SELECT __path AS path,__start AS start_byte,__end AS end_byte,'+','.join('coalesce('+col+",'') AS "+col for col in group)+' FROM (SELECT *,row_number() OVER(PARTITION BY __path,'+','.join(group)+' ORDER BY __start,__end) AS chosen FROM events) WHERE chosen=1'
  clauses.append({'id':id,'relational':{'inputs':{'events':input_},'query':query,'key':group},'violation':{'code':code,'message':id,'anchor':'witness.span'}})
 event('queue-auth-definitions',A(K('function_definition'),F('name',names(m.FORBIDDEN_QUEUE_AUTH_NAMES)),nottest),'SPT-001')
 event('queue-auth-calls',A(call(m.FORBIDDEN_QUEUE_AUTH_NAMES),nottest),'SPT-001')
 event('terminal-hydration-definitions',A(K('function_definition'),F('name',names(m.FORBIDDEN_TERMINAL_HYDRATION_HELPER_NAMES))),'SPT-002')
 event('large-column-reentry',A(call(m.LARGE_COLUMN_HELPER_NAMES|m.FORBIDDEN_TERMINAL_HYDRATION_HELPER_NAMES),pure),'SPT-002')
 event('large-column-hydration',A(call(['convert_row_to_model']),pure,args(kw('hydrate_large_columns',K('true')))),'SPT-002')
 event('direct-planning-io',A(call(m.DIRECT_IO_NAMES),pure,N(preprocess)),'SPT-003')
 event('query-planning-io',A(tail_rule(m.FORBIDDEN_QUERY_CALL_TAILS),pure,N(preprocess),nottest),'SPT-003')
 event('preprocess-io',A(preprocess,nottest,O(call(m.DIRECT_IO_NAMES),A(K('call'),F('function',names(m.PREPROCESS_SQL_EXECUTION_NAMES))),tail_rule(m.FORBIDDEN_QUERY_CALL_TAILS|m.PREPROCESS_SQL_EXECUTION_CALL_TAILS))),'SPT-011')
 event('provider-io',A(provider,nottest,O(call(m.DIRECT_IO_NAMES),A(K('call'),F('function',names(m.PREPROCESS_SQL_EXECUTION_NAMES))),tail_rule(m.FORBIDDEN_QUERY_CALL_TAILS|m.PREPROCESS_SQL_EXECUTION_CALL_TAILS))),'SPT-019')
 event('nested-plan-execution',A(call(m.FORBIDDEN_EXECUTION_TIME_REPLANNING_NAMES),pure),'SPT-005')
 event('executor-plan-execution',A(tail_rule(m.FORBIDDEN_EXECUTION_TIME_REPLANNING_CALL_TAILS),pure,nottest),'SPT-005')
 event('queue-udf-symbols',A(literal(m.FORBIDDEN_SQLITE_QUEUE_UDF_STRINGS),nottest),'SPT-004')
 event('materialized-pipeline',A(K('attribute'),F('attribute',names(['materialized_pipeline'])),pure),'SPT-012')
 event('context-authority',A(K('attribute'),F('object',names(['ctx'])),F('attribute',names(m.COMPILATION_CONTEXT_AUTHORITY_ATTRS)),symbolic,nottest),'SPT-020')
 resolved=A(K('identifier'),RX(r'^(?:resolved|resolved_refs)$'))
 # The source receiver predicate also recognizes attribute leaves with these names.
 resolved=O(resolved,A(K('attribute'),F('attribute',resolved)),A(N(O(K('identifier'),K('attribute'))),RX('resolved')))
 event('resolved-ref-access',A(K('call'),F('function',A(K('attribute'),F('attribute',names(m.FORBIDDEN_RESOLVED_RUNTIME_ACCESS_METHODS)),F('object',resolved))),pure),'SPT-010')
 ir=A(K('identifier'),RX(r'^(?:StepIR|PipelineResolvedIR|[A-Za-z_]\w*IR)$'),N(names(m.ALLOWED_PRODUCTION_IR_TYPE_NAMES)))
 event('production-ir-class',A(K('class_definition'),F('name',N(names(m.ALLOWED_PRODUCTION_IR_TYPE_NAMES))),F('superclasses',H(O(term(['StepIR','PipelineResolvedIR']),H(term(['StepIR','PipelineResolvedIR']),stop='end')))),nottest),'SPT-006')
 event('production-ir-construction',A(K('call'),F('function',O(ir,A(K('attribute'),F('attribute',ir)))),nottest),'SPT-006')
 compiled=call(['CompiledStep'])
 sql_ir=A(K('call'),F('function',term(['SqlStepIR'])))
 event('compiled-ir-value',A(compiled,nottest,args(kw('ir',A(P('IR'),N(sql_ir))))),'SPT-006','capture.IR')
 event('test-only-acknowledgement',A(K('call'),nottest,args(A(K('keyword_argument'),F('name',names(['tripwire_acknowledged_test_only']))))),'SPT-006')
 event('executor-ir-dispatch',A(call(['isinstance']),{'filePath':{'regex':'^(?:'+'|'.join(re.escape(v) for v in sorted(m.EXECUTOR_IR_DISPATCH_FILES))+')$'}},arg(2,O(A(ir,P('IR_NAME')),H(A(ir,P('IR_NAME')),stop='end'))),nottest),'SPT-006','capture.IR_NAME',each='IR_NAME')
 marker_fn=A(K('function_definition'),F('body',H(call(m.STEP_EXECUTION_MARKER_NAMES),stop='end')))
 event('step-aware-eager-helper',A(call(m.DIRECT_IO_NAMES),{'filePath':{'regex':'^(?:'+'|'.join(re.escape(v) for v in m.STEP_EXECUTION_EAGER_HELPER_PREFIXES)+')'}},I(marker_fn,K('function_definition')),nottest),'SPT-025')
 dc=A(K('class_definition'),F('name',O(names(['GraphSearchReadConfig','ContainmentEdgesReadConfig']),A(RX('Graph'),RX('Config$'),{'filePath':{'regex':'graph_read'}}))),I(A(K('decorated_definition'),H(A(K('decorator'),H(O(term(['dataclass']),call(['dataclass'])))))), 'neighbor'))
 ref_atom=A(K('identifier'),RX('ServerRef$'))
 broad=O(annword(['Any','LazyResult','object','str']),N(O(ref_atom,H(ref_atom,stop='end'))))
 event('graph-id-config',A(K('assignment'),F('left',A(K('identifier'),RX('graph_id'),P('FIELD'))),F('type',broad),I(dc,O(K('class_definition'),K('function_definition')))),'SPT-032','capture.FIELD')
 event('legacy-preop',A(compiled,nottest,args(A(K('keyword_argument'),F('name',names(['pre_op','async_pre_op']))))),'SPT-008')
 ddl=O(A(K('string'),{'value':{'regex':r'(?i)^\s*(?:CREATE|ALTER|DROP)\b'}}),A(K('call'),F('function',O(A(K('identifier'),RX(r'^Sql(?:Create|Alter|Drop)')),A(K('attribute'),F('attribute',RX(r'^Sql(?:Create|Alter|Drop)')))))))
 event('serverplan-ddl',A(call(m.FORBIDDEN_SERVERPLAN_DDL_HELPERS),nottest,O(A(K('call'),F('function',O(A(K('identifier'),RX('^ddl_')),A(K('attribute'),F('attribute',RX('^ddl_')))))),arg(1,O(ddl,H(ddl,stop='end'))))),'SPT-009')
 event('mcp-default-graph',A(call(['resolve_defaults']),I(A(K('function_definition'),F('name',names(m.MCP_DEFAULT_GRAPH_METHOD_NAMES))),K('function_definition'))),'SPT-034')
 graph_path={'filePath':{'regex':'(?:'+'|'.join(re.escape(v) for v in m.GRAPH_READ_REF_PRIVATE_ACCESS_PATH_SUFFIXES)+')$'}}
 event('private-lazy-ref',A(graph_path,O(call(['_lazy_result_step_ref_id']),A(K('call'),F('function',A(K('attribute'),F('attribute',names(['_step_ref_id']))))),A(K('call'),F('function',names(['getattr'])),arg(2,literal(['_step_ref_id']))))),'SPT-033')
 builder=A(K('call'),F('function',term(['graph_query_builder_lazy'])))
 chain=O(builder,A(K('attribute'),F('object',{'matches':'state-graph-chain'})),A(K('call'),F('function',{'matches':'state-graph-chain'})))
 utils['state-graph-chain']=chain
 event('graph-terminal-chain',A(K('call'),F('function',A(K('attribute'),F('attribute',names(m.GRAPH_TERMINAL_ENQUEUE_NAMES)),F('object',{'matches':'state-graph-chain'})))),'SPT-031')
 nested=O(call(m.NESTED_LAZY_BUILDER_NAMES|m.GRAPH_TERMINAL_ENQUEUE_NAMES),A(K('attribute'),F('attribute',names(['_step_ref_id']))))
 event('lazy-callback-terminal',A(call(m.LAZY_CALLBACK_COMPOSITION_NAMES),O(F('arguments',H(A(N(K('keyword_argument')),N({'nthChild':{'position':1,'ofRule':N(K('comment'))}}),O(nested,H(nested,stop='end'))))),args(A(K('keyword_argument'),F('name',names(['fn','func','callback','transform'])),F('value',O(nested,H(nested,stop='end'))))))),'SPT-030')
 from .state_signatures import signature_clauses
 clauses.extend(signature_clauses(m))
 from .state_flow import flow_clauses
 clauses.extend(flow_clauses(m))
 utils['state-project-source']=A(K('module'),RX('|'.join(re.escape(v) for v in sorted(m.STATE_TRIPWIRE_SCAN_TOKENS))))
 utils['state-function-source']=O(A(K('module'),RX('|'.join(re.escape(v) for v in sorted(m.FUNCTION_FACT_SCAN_TOKENS)))),A(K('module'),{'filePath':{'regex':'^'+re.escape(m.BACKEND_PROVIDER_PURITY_FILE)+'$'}}))
 step_base=A(K('class_definition'),F('superclasses',H(A(N(K('keyword_argument')),O(RX('ServerPlanStep|IntentStep'),A(strings,{'value':{'regex':'ServerPlanStep|IntentStep'}}))))))
 utils['state-signature-source']=A(K('module'),H(step_base,stop='end'))
 flow_root=O(pure_fn,A(K('function_definition'),O(F('name',names(['preprocess'])),F('name',RX('sql_builder$')),F('parameters',H(O(names(['runtime_refs']),RX('RuntimeBuilderSpec')),stop='end')),H(call(['CompiledStep']),stop='end'))))
 utils['state-flow-source']=A(K('module'),O({'filePath':{'regex':r'(?:^|/)(?:impl/common/l2070_server_plan/|'+re.escape(m.BACKEND_PROVIDER_PURITY_FILE)+r'$)'}},H(flow_root,stop='end')))
 utils['state-projection-source']=A(K('module'),H(A(O(K('function_definition'),K('class_definition')),F('name',RX('|'.join(re.escape(v) for v in sorted(m.PROJECTION_RECOVERY_SCOPE_TOKENS))))),stop='end'))
 source_gate=O({'matches':'state-project-source'},I({'matches':'state-project-source'}))
 function_gate=O({'matches':'state-function-source'},I({'matches':'state-function-source'}))
 for clause in clauses:
  if 'relational' not in clause:continue
  for relation in clause['relational']['inputs'].values():
   ast=relation['evidence']['query']['ast'];ast['rule']=A(choice('oracleAnalyzer',TRUE,source_gate),ast['rule'])
   if clause['id'].startswith('state-signatures'):
    ast['rule']=A(O({'matches':'state-signature-source'},I({'matches':'state-signature-source'})),ast['rule'])
   if clause['id'].startswith('state-projection-recovery'):
    ast['rule']=A({'filePath':{'regex':'|'.join(re.escape(v) for v in sorted(m.PROJECTION_RECOVERY_PATH_TOKENS))}},O({'matches':'state-projection-source'},I({'matches':'state-projection-source'})),ast['rule'])
   if clause['id'].startswith('state-flow'):
    ast['rule']=A({'$option':'entryPoint','cases':{'scan_project_sources':TRUE},'default':O({'matches':'state-flow-source'},I({'matches':'state-flow-source'}))},ast['rule'])
    ast['rule']=A({'$option':'entryPoint','cases':{'scan_project_sources':TRUE},'default':choice('sourceProvided',function_gate,TRUE)},ast['rule'])
 return clauses,utils
