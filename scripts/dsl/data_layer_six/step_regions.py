# SPDX-License-Identifier: Apache-2.0
"""Select the syntax needed by each step analysis before projecting graph facts."""
from .common import *

def metadata():
 bases=A(K('argument_list'),I(K('class_definition'),'neighbor'))
 return O(K('module'),A(K('expression_statement'),O(I(K('module'),'neighbor'),I(A(K('block'),I(K('class_definition'),'neighbor')),'neighbor'))),K('class_definition'),K('function_definition'),K('decorated_definition'),
  bases,I(bases,'neighbor'))

def family(entry):
 if entry in {'check_compile_callbacks_no_runtime_authority','check_postop_no_resolved_data_manipulation'}:return 'callbacks'
 if entry=='check_closed_post_op_runtime_builder_no_callable_escape_hatches':return 'builders'
 if entry=='check_legacy_postop_user_callable_escape_hatches':return 'legacy'
 if entry=='check_compiled_result_builders_do_not_fail_closed':return 'exceptions'
 if entry in {'check_runtime_ref_closed_post_op_consumption_guardrail','check_runtime_refs_post_op_requires_runtime_refs_guardrail','check_helper_anchored_runtime_ref_post_op_guardrail','check_legacy_postop_user_callable_escape_hatches','check_compiled_result_builders_do_not_fail_closed'}:return 'returns'
 if entry in {'check_row_materializer_raw_row_shaping_guardrail','check_literal_payload_materializer_dispatch_guardrail'}:return 'materializers'
 return 'structure'

def regions(group):
 meta=metadata()
 assignment=K('assignment')
 root_assignment=O(assignment,I(assignment,'neighbor'))
 signature=O(I(K('function_definition'),field='parameters'),I(K('function_definition'),field='return_type'))
 if group=='structure':
  contracts=['level','tables_read','tables_written','execution_role','cte_output_columns','cte_column_type_hints','step_tags','supports_sql_batch_transport','emits_sql_for_accounting','is_executor_owned_runtime_step']
  private_assignment=A(assignment,F('left',A(K('attribute'),F('object',names(['self'])),F('attribute',RX('^_')))))
  alias_assignment=A(assignment,I(fn('compile')),F('left',K('identifier')),F('right',A(K('attribute'),F('object',names(['self'])),F('attribute',RX('^_')))))
  state_assignment=O(private_assignment,alias_assignment)
  compiled=call(['CompiledStep'])
  argument_list=A(K('argument_list'),I(compiled,'neighbor'))
  kw=A(K('keyword_argument'),I(argument_list,'neighbor'))
  kw_container=A(O(K('tuple'),K('list'),K('set')),I(kw,'neighbor','value'))
  symbols=call(['runtime_builder_post_op','RuntimeBuilderSpec','scalar_column_post_op'])
  condition=A(O(K('if_statement'),K('elif_clause')),I(fn('compile')))
  contract_fn=A(K('function_definition'),F('name',names(contracts)))
  contract_return=A(K('return_statement'),I(contract_fn))
  returned_container=A(O(K('tuple'),K('list'),K('set')),I(contract_return,'neighbor'))
  frozen=call(['frozenset'])
  frozen_args=A(K('argument_list'),I(frozen,'neighbor'))
  frozen_container=A(O(K('tuple'),K('list'),K('set')),I(frozen_args,'neighbor'))
  contract_values=A(I(contract_fn),O(contract_return,I(contract_return,'neighbor'),I(returned_container,'neighbor'),
   I(frozen,'neighbor'),frozen_args,I(frozen_args,'neighbor'),I(frozen_container,'neighbor'),I(contract_fn,field='return_type')))
  derived_hints=A(fn('cte_column_type_hints'),O(RX('cte_output_columns'),H(O(K('list_comprehension'),K('set_comprehension'),K('dictionary_comprehension'),K('generator_expression'),K('conditional_expression')),stop='end')))
  return O(meta,contract_values,
   I(derived_hints),
   state_assignment,I(state_assignment,'neighbor'),
   condition,I(condition,field='condition'),
   compiled,I(compiled,'neighbor'),I(argument_list,'neighbor'),I(kw,'neighbor'),I(kw_container,'neighbor'),
   symbols,I(symbols),
   A(K('raise_statement'),I(fn('compile'))),I(A(K('raise_statement'),I(fn('compile')))))
 if group=='callbacks':
  nested=A(K('function_definition'),I(fn('compile')))
  callbacks_call=A(K('call'),I(nested))
  compiled=A(call(['CompiledStep']),I(fn('compile')))
  interesting=O(callbacks_call,compiled)
  arguments=A(K('argument_list'),I(interesting,'neighbor'))
  kw=A(K('keyword_argument'),I(arguments,'neighbor'))
  assign=A(assignment,I(fn('compile')),F('left',O(K('identifier'),K('attribute'))))
  authority_attrs=A(K('attribute'),F('attribute',names(['txn','_txn','conn','_conn','sql_conn','_serverplan_runtime_txn','_serverplan_sql_conn'])),I(nested))
  authority_types=RX('ReadTransaction|WriteTransaction|SqlConnection|DatabaseConnection')
  return O(meta,I(nested,field='parameters'),I(A(K('function_definition'),F('return_type',authority_types)),field='return_type'),
   interesting,I(interesting,'neighbor'),I(arguments,'neighbor'),I(kw,'neighbor'),
   assign,I(assign,'neighbor'),authority_attrs)
 if group=='builders':
  interesting=O(call(['runtime_builder_post_op','runtime_op_post_op','RuntimeBuilderSpec']),A(K('call'),F('function',A(RX('(?i)runtime'),RX('(?i)register'),RX('(?i)callback|callable|effect|function|hook|processor|transform')))))
  assigned=A(assignment,O(F('right',K('lambda')),F('left',A(RX('(?i)runtime'),RX('(?i)callback|callable|effect|function|hook'))),I(A(K('class_definition'),F('name',names(['RuntimeBuilderKind']))))))
  return O(meta,assigned,I(assigned,'neighbor'),interesting,I(interesting))
 if group=='materializers':
  rawparam=O(A(K('identifier'),RX('^(?:raw|raw_rows|rows)$')),A(O(K('typed_parameter'),K('typed_default_parameter')),RX('RawResult')))
  rawfn=A(K('function_definition'),O(F('parameters',H(rawparam)),F('name',names(['materialize']))))
  payload=A(K('call'),F('arguments',H(A(K('keyword_argument'),F('name',names(['row_materializer','literal_payload','terminal_literal']))))))
  return O(meta,I(rawfn),payload,I(payload))
 if group in ('legacy','exceptions'):
  compiled=call(['CompiledStep'])
  interesting=O(compiled,call(['ResultAssertionSpec','ResultMaterializationSpec','static_exception_post_op','assert_result','materialize']))
  returned_call=A(K('call'),I(K('return_statement'),'neighbor'))
  calls=O(interesting,returned_call)
  arguments=A(K('argument_list'),I(calls,'neighbor'))
  keyword=A(K('keyword_argument'),I(arguments,'neighbor'))
  frames=O(calls,I(calls,'neighbor'),I(arguments,'neighbor'),I(keyword,'neighbor'))
  if group=='legacy':
   shaped=A(K('call'),{'matches':'step-user-callable'})
   strict=A(assignment,F('left',names(['CLOSED_POST_OP_STRICT'])))
   return O(meta,frames,shaped,I(shaped),strict,I(strict,'neighbor'))
  params_for_spec=A(K('function_definition'),H(A(K('call'),F('function',term(['ResultAssertionSpec'])),F('arguments',H(A(K('keyword_argument'),F('name',names(['static_exception'])),F('value',K('identifier')))))),stop='end'))
  returned=A(K('return_statement'),H(K('call')))
  return O(meta,frames,returned,I(params_for_spec,field='parameters'),
   I(A(K('keyword_argument'),F('name',names(['closed_post_op']))),field='value'),
   call(['ResultAssertionSpec','static_exception_post_op','assert_result']),I(call(['ResultAssertionSpec','static_exception_post_op','assert_result'])),
   K('raise_statement'),I(K('raise_statement')))
 raise ValueError(group)

def user_callable_utility(m):
 shaped={'matches':'step-user-callable'}
 token=RX('(?i)'+'|'.join(re.escape(v) for v in sorted(m.USER_CALLABLE_ESCAPE_NAMES)))
 return O(K('lambda'),A(K('identifier'),token),A(K('attribute'),O(F('attribute',token),F('object',shaped))),
  A(K('call'),O(F('function',shaped),F('arguments',H(O(shaped,A(K('keyword_argument'),F('value',shaped))))))),
  A(K('subscript'),F('value',shaped)),A(K('parenthesized_expression'),H(shaped)))

def strict_module_utility():
 return A(K('module'),H(A(K('expression_statement'),H(A(K('assignment'),F('left',names(['CLOSED_POST_OP_STRICT'])),F('right',K('true')))))))
