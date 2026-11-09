"""Transport source arguments to the native ServerPlan step contracts."""
import argparse
import ast
import json
from pathlib import Path
import sys

from serverplan_steps_contract import FUNCTIONS

SETTINGS = ('LINTER_NAME','ROOT','STEP_DIRS','OBJECT_MANIPULATION_METHODS',
    'POSTOP_MANIPULATION_METHODS','CAN_BATCH_FALSE_ALLOWLIST','LIFECYCLE_RUNTIME_STATE_ALLOWLIST',
    'ALLOWLIST_SIGNATURE','RUNTIME_STATE_ATTRS','CALLBACK_RUNTIME_AUTHORITY_ATTRS',
    'CALLBACK_RUNTIME_AUTHORITY_HELPERS','AUTHORITY_TYPE_NAMES','LEGACY_SIDE_EFFECT_TAG_ALLOWLIST')


def install_settings(suite,module):
    """Replace imported configuration values with one typed native result batch."""
    rows=suite.native('serverplan_steps','_module_settings',[],dict(settingsRoot=str(suite.source)))
    kinds={};values={}
    for row in rows:
        p=row['parameters'];name=p['name']
        if name not in SETTINGS:raise ValueError('Unknown native module setting: '+name)
        if row['code']=='SSP_SETTING_TYPE':
            if name in kinds:raise ValueError('Duplicate native setting type')
            kinds[name]=p['kind']
        elif row['code']=='SSP_SETTING':values.setdefault(name,[]).append((p['ordinal'],p['value']))
        else:raise ValueError('Unexpected native module setting row')
    if set(kinds)!=set(SETTINGS):raise ValueError('Incomplete native module settings')
    for name,kind in kinds.items():
        pairs=sorted(values.get(name,[]));items=[value for _,value in pairs]
        if [i for i,_ in pairs]!=list(range(len(pairs))):raise ValueError('Invalid native setting order')
        if kind=='frozenset':value=frozenset(items)
        elif kind=='paths':value=tuple(Path(item) for item in items)
        elif kind in {'str','path'}:
            if len(items)!=1:raise ValueError('One scalar setting required')
            value=Path(items[0]) if kind=='path' else items[0]
        else:raise ValueError('Unknown native setting type')
        suite.replace(module,name,value)


def legacy_rows(rows):
    if any(row['code'] != 'SSP_LEGACY' for row in rows):
        raise ValueError('Unexpected native legacy scan result')
    ordered = sorted(rows,key=lambda r:tuple(r['parameters'][k] for k in
        ['root_order','path_order','phase','call_order','keyword_order','node_order','priority']))
    return [dict(code=r['parameters']['violation_code'],file=r['parameters']['file_path'],
                 line=r['parameters']['line_number'],message=r['parameters']['category']) for r in ordered]


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
        return dict(rootPath=str(module.ROOT),env=['ssp.api=true'], canBatchAllowlist=list(module.CAN_BATCH_FALSE_ALLOWLIST),
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

    def blockers(entry):
        directories = [str(path) for path in module.STEP_DIRS]
        environment = ['ssp.root='+str(module.ROOT), 'ssp.cwd='+str(Path.cwd()), 'ssp.collector='+entry]
        environment += ['ssp.dir'+str(i)+'='+str(Path(path).absolute()) for i,path in enumerate(directories)]
        environment += ['ssp.step.'+str(i)+'='+path for i,path in enumerate(directories)]
        rows = suite.native('serverplan_steps', entry, [],
                            dict(options(), stepDirs=directories, cwdPath=str(Path.cwd()), env=environment))
        if any(row['code'] != 'SSP_BLOCKER' for row in rows):
            raise ValueError('Unexpected native blocker result')
        return {row['parameters']['value'] for row in rows}

    def legacy_scan():
        directories = [str(path) for path in module.LEGACY_POSTOP_SCAN_DIRS]
        environment = ['ssp.root='+str(module.ROOT),'ssp.cwd='+str(Path.cwd()),'ssp.collector=scan_legacy_postop_violations']
        environment += ['ssp.dir'+str(i)+'='+str(Path(path).absolute()) for i,path in enumerate(directories)]
        environment += ['ssp.step.'+str(i)+'='+path for i,path in enumerate(directories)]
        rows = suite.native('serverplan_steps','scan_legacy_postop_violations',[],dict(options(),env=environment))
        return [module.Violation(v['code'],Path(v['file']),v['line'],v['message']) for v in legacy_rows(rows)]

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

    def cli_report(entry, values=(), **settings):
        rows = suite.native('serverplan_steps',entry,[],dict(rootPath=str(module.ROOT),
            reportCodes=[v['code'] for v in values],reportPaths=[v['file'] for v in values],
            reportLines=[str(v['line']) for v in values],reportMessages=[v['message'] for v in values],**settings))
        if any(row['code'] not in {'SSP_OUTPUT','SSP_EXIT'} for row in rows):
            raise ValueError('Unexpected native CLI result')
        exits = [row['parameters']['status'] for row in rows if row['code'] == 'SSP_EXIT']
        if len(exits) != 1:
            raise ValueError('One native CLI exit status required')
        for row in sorted((row['parameters'] for row in rows if row['code'] == 'SSP_OUTPUT'),
                          key=lambda row:(row['stage'],row['ordinal'])):
            sys.stdout.write(row['text'])
        return exits[0]

    def sign_allowlists():
        rows=suite.native('serverplan_steps','_sign_request',[],options())
        if len(rows)!=1 or rows[0]['code']!='SSP_SIGN_REQUEST':
            raise ValueError('One native signing request required')
        request=rows[0]['parameters']
        replies=suite.sign_ed25519([dict(id=request['request_id'],key_env=request['key_env'],message=request['message'])])
        if len(replies)!=1 or replies[0]['id']!=request['request_id']:
            raise ValueError('One matching native signing reply required')
        reply=replies[0];path=Path(module.__file__)
        if reply['status']=='signed':
            source=path.read_text(encoding='utf-8')
            result=suite.native('serverplan_steps','_sign_update',[],dict(scriptSource=source,scriptPath=str(path),newSignature=reply['signature']))
        else:
            result=suite.native('serverplan_steps','_sign_error',[],dict(signStatus=reply['status'],signDetail=reply['detail']))
        if len(result)!=1 or result[0]['code']!='SSP_SIGN_RESULT':
            raise ValueError('One native signing result required')
        value=result[0]['parameters']
        if value['write_file']:path.write_text(value['updated_source'],encoding='utf-8')
        sys.stdout.write(value['stdout']);sys.stderr.write(value['stderr'])
        return value['status']

    def cli():
        parser = argparse.ArgumentParser(description='Lint ServerPlan step purity rules')
        parser.add_argument('--check',action='store_true',help='Exit non-zero on violations')
        parser.add_argument('--sign-allowlist',action='store_true',help='Sign the current allowlists (requires NERA_SIGNING_KEY)')
        parser.add_argument('--verbose',action='store_true',help='Print scanned files')
        parser.add_argument('--scan-legacy-postops',action='store_true',help='Report legacy post-op behavior categories without failing by default')
        parser.add_argument('--legacy-postop-ratchet-max',type=int,default=None,help='Fail when the legacy post-op scan reports more than this many findings')
        args = parser.parse_args()
        if args.sign_allowlist:
            return sign_allowlists()
        if not module._verify_allowlists():
            return cli_report('_cli_signature_failure')
        if args.scan_legacy_postops:
            values = [dict(code=v.code,file=str(v.file),line=v.line,message=v.message)
                      for v in module.scan_legacy_postop_violations()]
            ratchet = {} if args.legacy_postop_ratchet_max is None else dict(legacyRatchet=str(args.legacy_postop_ratchet_max))
            return cli_report('_cli_legacy_report',values,**ratchet)
        directories = [str(path) for path in module.STEP_DIRS]
        extras = [str(path) for path in module.SERVERPLAN_EXTRA_SCAN_FILES]
        environment = ['ssp.root='+str(module.ROOT),'ssp.cwd='+str(Path.cwd()),'ssp.collector=_cli_paths']
        environment += ['ssp.dir'+str(i)+'='+str(Path(path).absolute()) for i,path in enumerate(directories)]
        environment += ['ssp.dirExtra'+str(i)+'='+str(Path(path).absolute().parent) for i,path in enumerate(extras)]
        environment += ['ssp.step.'+str(i)+'='+path for i,path in enumerate(directories)]
        environment += ['ssp.extra.'+str(i)+'='+path for i,path in enumerate(extras)]
        rows = suite.native('serverplan_steps','_cli_paths',[],dict(env=environment))
        if any(row['code'] != 'SSP_PATH' for row in rows):
            raise ValueError('Unexpected native CLI path result')
        paths = [row['parameters']['file_path'] for row in sorted(rows,key=lambda row:tuple(
            row['parameters'][k] for k in ['phase','root_order','path_order']))]
        # Discovery and classification stay native. This adapter transports raw
        # bytes as one batch and replays independent scanner-call multiplicity.
        unique = list(dict.fromkeys(paths))
        files = [dict(path='input/'+str(i)+'.py',source=(Path(path) if Path(path).is_absolute()
                    else module.ROOT/path).read_text(encoding='utf-8')) for i,path in enumerate(unique)]
        rows = suite.native('serverplan_steps','_cli_scan',files,dict(options(),cliPaths=unique))
        findings = scan_rows(rows)
        values = [value for path in paths for value in findings if value['file'] == path]
        return cli_report('_cli_report',values,verbosePaths=paths,cliVerbose=args.verbose,cliCheck=args.check)

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
        if any(row['code'] not in {'SSP_ROW','SSP_ERROR'} for row in rows):
            raise ValueError('Unexpected native step result')
        selected = [row for row in rows if row['parameters']['entry'] == entry]
        if any(row['code'] == 'SSP_ERROR' for row in selected):
            scan_rows(selected)
        result = []
        for row in sorted(selected, key=lambda r: tuple(r['parameters'][k] for k in
                          ['class_order', 'method_order', 'callback_order', 'node_order', 'suborder'])):
            if row['code'] != 'SSP_ROW':
                raise ValueError('Unexpected native step DTO: '+row['code'])
            p = row['parameters']
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
        elif name == 'scan_legacy_postop_violations':
            suite.replace(module,name,legacy_scan)
        elif name == 'cli_main':
            suite.replace(module,name,cli)
        elif name in {'collect_current_step022_blockers', 'collect_current_step023_blockers'}:
            def bind_blockers(entry):
                return lambda: blockers(entry)
            suite.replace(module, name, bind_blockers(name))
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
