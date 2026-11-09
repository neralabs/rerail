"""Transport source arguments to the native ServerPlan step contracts."""
import ast
import json
from pathlib import Path
import sys

from serverplan_steps_contract import FUNCTIONS


def scan_rows(rows):
    """Decode the native scanner's ordered DTOs and error result."""
    errors = [row for row in rows if row['code'] == 'SSP_ERROR']
    if errors:
        if len(rows) != 1:
            raise ValueError('A scanner error must be the only native result')
        raise ValueError(errors[0]['parameters']['detail'])
    if any(row['code'] != 'SSP_ROW' for row in rows):
        raise ValueError('Unexpected native scanner result')
    ordered = sorted(rows, key=lambda row: tuple(row['parameters'][k] for k in
                     ['check_order', 'class_order', 'method_order', 'callback_order', 'node_order', 'suborder']))
    return [dict(code=row['parameters']['violation_code'], file=row['path'], line=row['line'],
                 message=row['parameters']['detail']) for row in ordered]


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
                    signature=module.ALLOWLIST_SIGNATURE,
                    fateNames=list(module.POST_OP_FATE_OVERRIDES),
                    fateValues=[value.value for value in module.POST_OP_FATE_OVERRIDES.values()])

    def tree_source(tree):
        if tree not in sources.origins:
            raise NotImplementedError('Constructed AST arguments need structured transport')
        origin = sources.origins[tree]
        if tree is not origin.tree or ast.dump(tree, include_attributes=True) != origin.snapshot:
            raise NotImplementedError('Changed AST arguments need structured transport')
        return origin.source

    def verify_allowlists():
        rows = suite.native('serverplan_steps', '_verify_allowlists', [], options())
        if len(rows) != 1 or rows[0]['code'] != 'SSP_VERIFY' or rows[0]['parameters']['ordinal'] != 0:
            raise ValueError('One native verification result required')
        result = rows[0]['parameters']
        sys.stdout.write(result['stdout'])
        sys.stderr.write(result['stderr'])
        return bool(result['valid'])

    def scan_source(file, source, tree=None):
        settings = dict(options(), argumentPath=str(file), callSource=source,
                        suppliedTree='0' if tree is None else '1')
        files = [] if tree is None else [dict(path='input.py', source=tree_source(tree))]
        rows = suite.native('serverplan_steps', '_scan_source', files, settings)
        return [module.Violation(v['code'], Path(v['file']), v['line'], v['message']) for v in scan_rows(rows)]

    def scan_file(file):
        absolute = file if file.is_absolute() else module.ROOT/file
        rows = suite.native('serverplan_steps', '_scan_file',
                            [dict(path='input.py', source=absolute.read_text(encoding='utf-8'))],
                            dict(options(), argumentPath=str(file), inputName=file.name))
        return [module.Violation(v['code'], Path(v['file']), v['line'], v['message']) for v in scan_rows(rows)]

    def check(entry, file, tree, **kwargs):
        source = tree_source(tree)
        settings = dict(options(), argumentPath=str(file),
                        strictModulesOnly=str(int(kwargs.get("strict_modules_only", True))))
        requested = getattr(suite, 'requested_entries', None)
        if requested is not None:
            if entry not in requested:
                raise ValueError('The original assertion did not request this detector: '+entry)
            settings['requestedChecks'] = requested
        key = (source, json.dumps(settings, sort_keys=True))
        if key not in batches:
            batches[key] = suite.native('serverplan_steps', '_all_checks',
                            [dict(path='input.py', source=source)], settings)
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
                   'check_compile_callbacks_no_runtime_authority',
                   'check_runtime_refs_post_op_requires_runtime_refs_guardrail',
                   'check_runtime_ref_closed_post_op_consumption_guardrail',
                   'check_helper_anchored_runtime_ref_post_op_guardrail',
                   'check_row_materializer_raw_row_shaping_guardrail',
                   'check_literal_payload_materializer_dispatch_guardrail',
                   'check_step_role_contract_guardrails',
                   'check_closed_post_op_runtime_builder_no_callable_escape_hatches',
                   'check_runtime_refs_without_sql_grammar_guardrail',
                   'check_compiled_result_builders_do_not_fail_closed',
                   'check_legacy_postop_user_callable_escape_hatches'}
    for name in FUNCTIONS:
        if name == '_verify_allowlists':
            suite.replace(module, name, verify_allowlists)
        elif name == '_scan_source':
            suite.replace(module, name, scan_source)
        elif name == '_scan_file':
            suite.replace(module, name, scan_file)
        elif name in implemented:
            def bind(entry):
                return lambda file, tree, **kwargs: check(entry, file, tree, **kwargs)
            suite.replace(module, name, bind(name))
        else:
            def bind_missing(entry):
                def missing(*args, **kwargs):
                    raise NotImplementedError('Native ServerPlan steps entry is not yet ported: '+entry)
                return missing
            suite.replace(module, name, bind_missing(name))
