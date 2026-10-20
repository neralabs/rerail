# SPDX-License-Identifier: Apache-2.0
"""Authored step purity, lifecycle, and result contract checks."""
from .common import *

def step_clauses(m):
 clauses=[]
 step=A(K('class_definition'),P('CLASS'),F('name',P('CLASS_NAME')),F('superclasses',H(O(names(['ServerPlanStep']),A(K('attribute'),F('attribute',names(['ServerPlanStep']))),A(K('subscript'),F('value',term(['ServerPlanStep'])))))))
 owner=lambda rule:I(rule,O(K('function_definition'),K('class_definition'),K('lambda')))
 method=lambda name:A(fn(name),P('METHOD'),O(I(A(K('block'),I(step,'neighbor')),'neighbor'),I(A(K('decorated_definition'),I(A(K('block'),I(step,'neighbor')),'neighbor')),'neighbor')))
 compile_=method('compile')
 compilebody=owner(compile_)
 body=lambda rule:F('body',H(rule,stop='end'))
 literal=lambda value:A(O(K('string'),K('concatenated_string')),{'value':{'regex':'^'+re.escape(value)+'$'}})
 keyword=lambda name,value=None:A(K('keyword_argument'),F('name',names([name])),F('value',value or P('VALUE')))
 compiled=A(K('call'),F('function',names(['CompiledStep'])))
 args=lambda rule:F('arguments',H(rule))
 missinggrammar=N(args(keyword('grammar',N(K('none')))))
 runtime_nonempty=N(O(K('none'),A(O(K('tuple'),K('list'),K('set')),N(H(P('_')))),literal('')))
 closed=A(K('call'),F('function',O(A(K('attribute'),F('attribute',RX(r'^(?:materialize|assert_result|.*_post_op)$'))),A(K('identifier'),RX(r'_post_op$')))))
 strict=I(A(K('module'),H(A(K('expression_statement'),H(A(K('assignment'),F('left',names(['CLOSED_POST_OP_STRICT'])),F('right',K('true'))))))))
 def c(id,rule,number,entry,anchor='witness.span',each=None):
  clauses.append(C(id,rule,f'STEP-{number:03d}',[entry,'_scan_source','_scan_file'],anchor,each))
 c('compile-object-manipulation',A(call(m.OBJECT_MANIPULATION_METHODS),compilebody),17,'check_compile_no_object_manipulation')
 c('cannot-batch',A(method('can_batch'),N(F('name',names(m.CAN_BATCH_FALSE_ALLOWLIST))),body(A(K('return_statement'),H(K('false')),I(K('block'),'neighbor')))),20,'check_can_batch_allowlist')
 runtime=A(K('attribute'),F('object',names(['self'])),F('attribute',names(m.RUNTIME_STATE_ATTRS)))
 c('runtime-state',A(runtime,I(A(K('assignment'),F('left',P('TARGET'))),'neighbor'),I(A(step,N(F('name',names(m.LIFECYCLE_RUNTIME_STATE_ALLOWLIST)))))),21,'check_runtime_state_allowlist')
 c('side-effect-tags',A(method('step_tags'),body(literal('side_effect')),
  N({'compare':{'left':'CLASS_NAME','right':'CLASS_NAME','op':'ne','text':'node'}})),22,'check_side_effect_tag_allowlist')
 # Class/path signed exceptions are applied to relation rows below when nonempty.
 if m.LEGACY_SIDE_EFFECT_TAG_ALLOWLIST:
  raise ValueError('Nonempty step side-effect inventory needs class/path evidence')
 c('misc-runtime-member',A(K('attribute'),F('object',names(['RuntimeBuilderKind'])),F('attribute',names(['MISC_RUNTIME']))),30,'check_misc_runtime_is_deleted')
 c('misc-runtime-definition',A(K('assignment'),N(F('type',P('_'))),F('left',names(['MISC_RUNTIME']))),30,'check_misc_runtime_is_deleted')
 value=A(O(K('string'),K('concatenated_string')),P('VALUE'))
 positional=lambda position,rule:args(A(rule,{'nthChild':{'position':position,'ofRule':N(K('comment'))}}))
 c('runtime-builder-string-name',O(A(call(['runtime_builder_post_op']),O(positional(2,value),A(N(positional(2,P('_'))),args(keyword('builder_name',value))))),
  A(call(['RuntimeBuilderSpec']),args(keyword('builder_name',value))),
  A(call(m.RUNTIME_BUILDER_HELPER_KINDS),O(positional(1,value),A(N(positional(1,P('_'))),args(keyword('builder_name',value)))))),31,'check_runtime_builder_names_are_enum_members','capture.VALUE')
 c('closed-migration-refs',A(compiled,strict,missinggrammar,args(keyword('runtime_ref_ids',P('VALUE')))),27,'check_closed_post_op_migration_guardrail','capture.VALUE')
 c('closed-migration-with-refs',A(compiled,strict,args(keyword('post_op_with_refs',P('VALUE')))),27,'check_closed_post_op_migration_guardrail','capture.VALUE')
 c('closed-migration-postop',A(compiled,strict,args(keyword('post_op',A(P('VALUE'),N(closed))))),27,'check_closed_post_op_migration_guardrail','capture.VALUE')
 c('terminal-projection-dispatch',A(K('comparison_operator'),{'filePath':{'regex':r'(?:^|/)terminal_projection_builders\.py$'}},H(names(['projection_kind'])),I(fn('materialize_terminal_projection'))),37,'check_terminal_projection_string_dispatch_guardrail')
 c('empty-result-assertion',A(K('call'),F('function',names(['ResultAssertionSpec'])),N(args(P('_')))),38,'check_empty_result_assertion_placeholder_guardrail')
 c('source-ref-ingress',O(A(call(['ResultMaterializationSpec']),args(keyword('source_refs'))),A(call(['ResultMaterializationSpec']),args(keyword('source_names'))),A(call(['ResultMaterializationSpec']),args(keyword('source_materializer'))),
  A(call(['runtime_refs_post_op']),args(keyword('source_refs'))),A(call(['CompiledStep','BatchChildResultSpec']),args(keyword('raw_payload_ref_id')))),42,'check_result_materialization_spec_no_source_ref_escape_hatch')
 parse=C('parse-error',K('module'),'PARSE',['_scan_source','_scan_file'],anchor='file.start')
 parse['forbid']['ast']['syntax']='invalid'
 clauses.append(parse)
 from .step_contracts import contract_clause
 clauses.extend(contract_clause(m))
 from .step_returns import return_clause
 clauses.append(return_clause())
 return clauses
