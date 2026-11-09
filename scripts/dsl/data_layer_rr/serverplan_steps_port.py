"""Transport source arguments to the native ServerPlan step contracts."""
import ast
import json
from pathlib import Path

from serverplan_steps_contract import FUNCTIONS


def install(suite, module):
    from ast_sources import Sources
    sources = Sources(ast.parse)
    suite.replace(ast, 'parse', sources.parse)

    def initialize(value, code, file, line, message):
        value.code, value.file, value.line, value.message = code, file, line, message
    suite.replace(module.Violation, '__init__', initialize)

    def render(value):
        rows = suite.native('serverplan_steps', 'Violation.__str__', [],
                            dict(argumentPath=str(value.file), rootPath=str(module.ROOT),
                                 violationCode=value.code, violationLine=str(value.line), detail=value.message))
        if len(rows) != 1 or rows[0]['code'] != 'SSP_VALUE':
            raise ValueError('One native formatted violation required')
        return rows[0]['parameters']['value']
    suite.replace(module.Violation, '__str__', render)
    batches = {}

    def options():
        return dict(canBatchAllowlist=list(module.CAN_BATCH_FALSE_ALLOWLIST),
                    runtimeStateAllowlist=list(module.LIFECYCLE_RUNTIME_STATE_ALLOWLIST),
                    sideEffectAllowlist=list(module.LEGACY_SIDE_EFFECT_TAG_ALLOWLIST),
                    signature=module.ALLOWLIST_SIGNATURE)

    def check(entry, file, tree):
        if tree not in sources.origins:
            raise NotImplementedError('Constructed AST arguments need structured transport')
        origin = sources.origins[tree]
        if tree is not origin.tree or ast.dump(tree, include_attributes=True) != origin.snapshot:
            raise NotImplementedError('Changed AST arguments need structured transport')
        settings = dict(options(), argumentPath=str(file))
        key = (origin.source, json.dumps(settings, sort_keys=True))
        if key not in batches:
            batches[key] = suite.native('serverplan_steps', '_all_checks',
                            [dict(path='input.py', source=origin.source)], settings)
        rows = batches[key]
        result = []
        for row in sorted(rows, key=lambda r: tuple(r['parameters'][k] for k in
                          ['class_order', 'method_order', 'callback_order', 'node_order', 'suborder'])):
            if row['code'] != 'SSP_ROW':
                raise ValueError('Unexpected native step DTO: '+row['code'])
            p = row['parameters']
            if p['entry'] != entry:
                continue
            result.append(module.Violation(p['violation_code'], Path(row['path']), row['line'], p['detail']))
        return result

    implemented = {'check_compile_no_data_branching', 'check_compile_no_object_manipulation',
                   'check_compile_must_not_raise', 'check_postop_no_resolved_data_manipulation',
                   'check_can_batch_allowlist', 'check_runtime_state_allowlist',
                   'check_side_effect_tag_allowlist', 'check_zero_io_l0_execution_role_guardrail',
                   'check_zero_io_l0_execution_role_enum_semantics',
                   'check_misc_runtime_is_deleted', 'check_runtime_builder_names_are_enum_members',
                   'check_runtime_builder_kind_name_pairs', 'check_closed_post_op_migration_guardrail',
                   'check_virtual_execute_runtime_scalar_value_guardrail',
                   'check_virtual_execute_runtime_python_shaping_guardrail',
                   'check_terminal_projection_string_dispatch_guardrail',
                   'check_empty_result_assertion_placeholder_guardrail',
                   'check_sql_visible_steps_declare_cte_type_hints',
                   'check_cte_type_hints_not_inferred_from_column_names',
                   'check_result_materialization_spec_no_source_ref_escape_hatch',
                   'check_compile_callbacks_no_runtime_authority'}
    for name in FUNCTIONS:
        if name in implemented:
            def bind(entry):
                return lambda file, tree: check(entry, file, tree)
            suite.replace(module, name, bind(name))
        else:
            def bind_missing(entry):
                def missing(*args, **kwargs):
                    raise NotImplementedError('Native ServerPlan steps entry is not yet ported: '+entry)
                return missing
            suite.replace(module, name, bind_missing(name))
