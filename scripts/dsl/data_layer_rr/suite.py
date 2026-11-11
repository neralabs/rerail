"""Run original pytest assertions with .rr decisions. Refuse original semantic calls."""
from __future__ import annotations
import argparse
import ast
import importlib
import inspect
import json
import os
import select
import subprocess
import sys
import tempfile
import threading
import textwrap
from functools import wraps
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

import pytest
import migration
from migration import ROOT, PIN, sha, git, write_json, compare, compare_inputs, fixtures_path
from dto import canonical

sys.path.insert(0, str(ROOT / 'scripts/dsl/data_layer_audit'))
from native_guard import codes, bind_aliases

ENTRIES = {
    'removed_compat_surfaces': ['scan_file'],
    'versioned_lifecycle_legacy_surfaces': ['scan_file', 'scan_file_content'],
    'no_source_inspection': ['find_violations', 'scan_file_content'],
    'serverplan_name_inference': ['violations', '_scan_source'],
    'permission_error': ['check_file', 'check_file_content'],
    'hasattr_async': ['check_file', 'check_source', 'is_allowed_line', 'should_exclude'],
    'no_blanket_grammar_bypass': ['check_no_bypass_references', 'check_no_public_grammar_methods', 'scan_file_content'],
    'admin_async_purity': ['_collect_violations', '_collect_violations_from_source', '_should_skip', '_is_allowlisted'],
    'async_executor': ['check_file', 'check_source', 'should_check_file', 'is_allowed'],
    'dynamic_module_loading': ['_scan_file', '_scan_file_content', 'find_violations', '_is_in_type_checking_block'],
    'classvar_check': ['lint_file', 'lint_file_content', 'lint_directory'],
    'type_safety_patterns': ['check_file_for_patterns', 'check_file_content_for_patterns', 'check_function_signatures',
        'check_function_signature_content', 'normalize_path', 'is_allowlisted', 'find_python_files', '_issues_for_files', '_dedupe_issues'],
    'forwardref': ['lint_file', 'lint_file_content', 'lint_directory'],
    'data_layer_facade_purity': ['lint_file', 'lint_targets'],
    'sync_async_signatures': ['analyze_file'],
    'sql_grammar_escape_hatches': [],
    'transaction_patterns': [],
    'plan_composition_architecture': [],
    'conn_or_txn_union': ['analyze_file'],
    'async_patterns': ['lint_file', 'analyze_source', 'should_exclude'],
    'async_sync_boundary': ['scan_tree', '_get_call_name', '_is_allowlisted', '_is_informational'],
    'plan_scope_ownership':['lint','_lint_file','_source_facts'],
    'backend_coupling': ['BackendCouplingChecker.check_file', 'BackendCouplingChecker.should_check_file'],
    'connection_caches': ['find_violations', 'scan_file_content', '_is_allowed'],
    'no_api_root_imports': ['check_file', 'check_file_content'],
    'error_handling': ['scan_file_content'],
    'backend_type_strings': ['check_file', 'check_file_content', 'should_skip_file', 'find_python_files'],
    'serverplan_grammar_surface': ['scan_file', 'iter_python_files'],
    'serverplan_target_architecture': ['scan_sources', '_frozen_slots', '_symbol_names'],
    'serverplan_state_tripwires': ['scan_source', 'scan_tree', 'scan_project_sources'],
    'legacy_execution_surfaces': ['scan_file_content', 'scan_sql_post_fetch_filter_content', 'scan_repo',
                                  'check_exact_file_inventory', 'check_removed_sql_read_leaf_inventory',
                                  '_iter_python_files', '_receiver_tokens', '_looks_like_cursor_receiver',
                                  '_is_graph_fallback_guard', '_is_under', '_get_allowlists_for_signing'],
    'unresolved_forward_refs': ['analyze_file', 'AnnotationUsageFinder.analyze', 'AnnotationUsageFinder._is_field_annotation_access',
        'AnnotationUsageFinder._is_safe_str_conversion', 'AnnotationUsageFinder._is_none_check',
        'AnnotationUsageFinder._is_isinstance_none_check', 'AnnotationUsageFinder._node_contains',
        'AnnotationUsageFinder._is_using_resolved_hints', 'AnnotationUsageFinder._is_assignment_from_resolved'],
    'hnsw_architecture': ['find_hnsw_architecture_findings'],
    'data_layer_compat_aliases': ['lint_file', 'lint_file_content', 'lint_targets', '_is_public_name', '_leaf_name'],
}


@dataclass(frozen=True)
class NativeRegexMatch:
    """Native counterpart for the match type and explicitly supplied capture assertions."""
    groups: dict

    def group(self, index):
        if index not in self.groups:
            raise AssertionError('No native DTO contract for this capture group')
        return self.groups[index]


class Suite:
    def __init__(self, source, engine, checks, part, parts, baseline, rule_root=None, execution_mode="snapshot"):
        if execution_mode not in {"snapshot","workspace"}:
            raise ValueError("Unknown native execution mode: "+execution_mode)
        self.execution_mode = execution_mode
        manifest = migration.read_json(ROOT / migration.MANIFEST)
        migration.verify_source_cohort(ROOT, manifest['engineSource'])
        for retired in ('.rerail/rr-migration-20261106/engine',
                        '.rerail/data-layer-batch-seven/engine-source'):
            if engine.resolve().is_relative_to((ROOT / retired).resolve()):
                raise ValueError('Acceptance cannot use a binary from a retired engine checkout')
        migration.verify_engine_artifact(ROOT, manifest['engineSource'], engine)
        self.source, self.engine, self.checks = source, engine, checks
        self.part, self.parts = part, parts
        self.rule_root = rule_root or ROOT / 'gate/data-layer/packs/data-layer/candidates/rr'
        self.originals, self.calls, self.cases, self.translations = [], [], {}, []
        self.collected = []
        self.semantic_fallbacks = []
        self.host_protocol_calls = []
        self.configuration_bindings = {}
        self.item = None
        self.inputs = {}
        raw = baseline.read_bytes()
        self.baseline = json.loads(raw)
        if (self.baseline['commit'] != PIN or self.baseline['pytest_exit_code'] != 0
                or self.baseline.get('source_mismatches')):
            raise ValueError('A successful pinned original recording is required')
        self.baseline_sha = sha(raw)
        self.observations, self.observed = [], Counter()
        self.cache = tempfile.TemporaryDirectory(prefix='rerail-rr-assertion-cache-')
        self.worker = subprocess.Popen([sys.executable, str(Path(__file__).with_name('replay.py')), '--workspace-worker' if execution_mode == 'workspace' else '--worker', str(engine)],
            text=True, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=ROOT,
            env={**os.environ, 'RERAIL_TAPE_CACHE': str(Path(self.cache.name) / 'store.sqlite')})

    def replace(self, module, name, value):
        self.originals.append((module, name, getattr(module, name)))
        setattr(module, name, value)

    def sign_ed25519(self,requests):
        from replay import sign_native
        return sign_native(self.engine,requests)

    def native(self, check, entry, files, options=None, *, root=None):
        from policy_routes import policy_filename
        rule = self.rule_root / policy_filename(check, entry)
        request = dict(rule=str(rule), files=files, options=dict(({} if entry is None else dict(entryPoint=entry)), **(options or {})))
        call = dict(test=self.item.nodeid if self.item else '<native-configuration>', check=check, entry=entry, policy=str(rule), files=len(files), status='started', executionMode=self.execution_mode,
                    inputFiles=[dict(path=f['path'], sourceSha256=sha(f['source'].encode())) for f in files],
                    options=request['options'])
        if root is not None:
            if files:raise ValueError('Repository transport takes physical resources, not document copies')
            request['root']=str(Path(root).absolute());call['repositoryRoot']=request['root']
        self.calls.append(call)
        self.worker.stdin.write(json.dumps(request) + '\n')
        self.worker.stdin.flush()
        if not select.select([self.worker.stdout], [], [], 110)[0]:
            self.worker.kill()
            raise RuntimeError('Native .rr assertion exceeded 110 seconds')
        line = self.worker.stdout.readline()
        if not line:
            raise RuntimeError('Native assertion worker exited: ' + self.worker.stderr.read())
        response = json.loads(line)
        if 'error' in response:
            call.update(status='failed', error=response['error'])
            raise AssertionError(response['error'])
        if response.get('executionMode') != self.execution_mode:
            call.update(status='failed',error='Native execution mode differs from the requested API')
            raise ValueError(call['error'])
        result = response['result']
        previous = self.inputs.setdefault((check, rule.name), result['programInputs'])
        if previous != result['programInputs']:
            raise AssertionError('Program changed during original assertions')
        rows = result['diagnostics']
        call.update(status='passed', findings=len(rows), diagnostics=rows,
                    programInputs=result['programInputs'], phases=result['phases'])
        return rows

    def compare_original(self, check, entry, module, args, kwargs, result, native_diagnostics=None):
        key = (self.item.nodeid, check, entry)
        records = [r for r in self.baseline['events'] if (r['test'], r['check'], r['entry']) == key]
        if not records:
            return
        index = self.observed[key]
        self.observed[key] += 1
        if index >= len(records):
            raise AssertionError('The original detector call count changed')
        original = records[index]
        if check == 'async_patterns' and entry == 'analyze_source':
            from ast_transport import syntax_columns
            values = inspect.signature(getattr(module, entry)).bind(*args, **kwargs).arguments
            tree = values.get('tree')
            compare(original['suppliedAst'], syntax_columns(tree) if tree is not None else None)
        observation = dict(test=key[0], check=check, entry=entry, ordinal=index,
                           originalRetainedReason=original.get('retained_reason'))
        self.observations.append(observation)
        from data_layer_record import ADAPTERS, snapshot, normalize
        if check=='plan_scope_ownership':
            from plan_ownership_port import snapshot
            files,scope,root=snapshot(module,entry,args,kwargs)
        elif check == 'unresolved_forward_refs' and entry == 'AnnotationUsageFinder.analyze':
            from unresolved_port import supplied_snapshot, analyzed_state
            owner = args[0]
            tree = args[1] if len(args) > 1 else kwargs['tree']
            files, scope, root, syntax = supplied_snapshot(owner, tree)
            compare(original['suppliedAst'], syntax)
            result = analyzed_state(owner, result)
        elif check == 'backend_type_strings' and entry in {'should_skip_file', 'find_python_files'}:
            files, scope, root = [], dict(include=[], exclude=[]), self.item.funcargs.get('tmp_path')
        elif check == 'hnsw_architecture':
            files, scope, root = snapshot(module, 'files', args, kwargs)
        elif check == 'data_layer_compat_aliases':
            from aliases_port import snapshot
            files, scope, root = snapshot(module, entry, args, kwargs, self.item, self.source)
        elif check == 'data_layer_facade_purity':
            from facade_purity_port import snapshot
            files, scope, root = snapshot(module, entry, args, kwargs)
        elif check == 'connection_caches':
            from connection_caches_port import snapshot
            files, scope, root = snapshot(module, entry, args, kwargs)
        elif check == 'error_handling':
            from error_handling_port import snapshot
            files, scope, root = snapshot(module, entry, args, kwargs)
        elif check == 'backend_coupling':
            from coupling_port import snapshot
            files, scope, root = snapshot(args[2], args[3])
        elif check == 'sync_async_signatures':
            files, scope, root = snapshot(module, 'file', args, kwargs)
        elif check == 'serverplan_target_architecture':
            from target_architecture_port import files_for
            sources = args[0] if args else kwargs['sources']
            files, scope, root = files_for(sources), dict(include=['**/*.py'], exclude=[]), None
        elif check == 'legacy_execution_surfaces':
            from legacy_execution_port import snapshot
            values = inspect.signature(getattr(module, entry)).bind(*args, **kwargs).arguments
            files, scope, root = snapshot(module, entry, values)
        elif check == 'serverplan_state_tripwires':
            from state_tripwires_port import snapshot
            values = inspect.signature(getattr(module,entry)).bind(*args,**kwargs).arguments
            files, scope, root = snapshot(module,values,entry)
        elif check == 'admin_async_purity':
            from data_layer_batch_two import normalize as normalize_two
            root = Path(args[0] if args else kwargs['project_root'])
            files = self.admin_files(module, root)
            scope = dict(include=['**/*.py'], exclude=[])
        else:
            from data_layer_batch_two import ADAPTERS as adapters_two, snapshot as snapshot_two
            kind = next(kind for c, e, kind in (*ADAPTERS, *adapters_two) if (c, e) == (check, entry))
            files, scope, root = (snapshot_two(module, entry, args, kwargs) if kind == 'batch_two'
                                  else snapshot(module, kind, args, kwargs))
        if 'sourceResult' not in original:
            raise AssertionError('A lossless original recording is required; use data_layer_rr/source.py')
        actual_source_result = canonical(result, root)
        observation.update(expectedSourceResult=original['sourceResult'], actualSourceResult=actual_source_result)
        compare(original['sourceResult'], actual_source_result)
        if check=='no_blanket_grammar_bypass' and original.get('retained_reason') and 'findings' not in original:
            # The old finding normalizer cannot represent a missing-file string.
            # Compare the complete source value and inputs instead of fabricating a finding.
            compare_inputs(original,files,scope)
            observation.update(expected=original['sourceResult'],actual=actual_source_result,status='passed',comparisonKind='source-value')
            return
        # A recorded empty list fully specifies this I/O result, even when the
        # older recorder withheld its redundant normalized finding list.
        empty_file_read=((check in {'backend_type_strings','hasattr_async'} and entry=='check_file'
                          or check=='no_blanket_grammar_bypass' and entry=='check_no_bypass_references') and not files
                         and original['sourceResult']==[] and actual_source_result==[] and original.get('findings',[])==[])
        empty_inspection=(check=='no_source_inspection' and entry=='find_violations' and not files
                          and original['sourceResult']==[] and actual_source_result==[] and original['findings']==[])
        if original.get('retained_reason') and not (empty_file_read or empty_inspection):
            observation['status'] = 'source-result-passed; legacy normalized DTO unavailable'
            return
        if check == 'backend_type_strings' and not empty_file_read and any(row['code'] == 'BTS_SKIPPED' for row in
                (self.calls[-1]['diagnostics'] if native_diagnostics is None else native_diagnostics)):
            scope = dict(include=[], exclude=[])
        compare_inputs(original, files, scope)
        from data_layer_batch_two import normalize as normalize_two
        if check == 'unresolved_forward_refs' and entry == 'AnnotationUsageFinder.analyze':
            actual = normalize_two(check, result['findings'], root, files)
        elif check == 'backend_type_strings' and entry in {'should_skip_file', 'find_python_files'}:
            actual = []
        elif check == 'connection_caches':
            from connection_caches_port import normalize
            actual = normalize(result, root)
        elif check == 'error_handling':
            from error_handling_port import normalize
            actual = normalize(result, root)
        elif check == 'backend_coupling':
            from data_layer_batch_five import normalize as normalize_five
            actual = normalize_five(check, result, root, files)
        elif check == 'data_layer_facade_purity':
            from data_layer_batch_five import normalize as normalize_five
            actual = normalize_five(check, result.violations if entry == 'lint_targets' else result, root, files)
        elif check == 'data_layer_compat_aliases':
            from data_layer_batch_five import normalize as normalize_five
            actual = normalize_five(check, result, root, files)
        elif check == 'sync_async_signatures':
            from signatures_port import legacy_message
            actual = [dict(path=files[0]['path'], line=v.async_func.line, column=None,
                code='sync_async_signature_mismatch', message=canonical(legacy_message(v), root)) for v in result]
        elif check in {'serverplan_target_architecture', 'legacy_execution_surfaces', 'serverplan_state_tripwires'}:
            from data_layer_batch_six import normalize as normalize_six
            actual = normalize_six(check, result, root, files)
        elif check == 'versioned_lifecycle_legacy_surfaces':
            # This DTO has no message field. Use the engine's complete diagnostic instead of
            # calling the source Violation.render() from the legacy recording normalizer.
            from replay import result as diagnostic_result
            call=self.calls[-1]
            owners={doc:canonical(Path(owner),root)['value'] for doc,owner in
                    zip(call['options'].get('ownerDocs',[]),call['options'].get('ownerPaths',[]))}
            actual = diagnostic_result(dict(check=check),
                [dict(row,path=owners.get(row['path'],row['path'])) for row in call['diagnostics']])
        else:
            actual = (normalize_two(check, result, root, files) if check in {'admin_async_purity', 'async_executor', 'dynamic_module_loading', 'classvar_check', 'type_safety_patterns', 'forwardref', 'unresolved_forward_refs', 'conn_or_txn_union', 'async_patterns', 'async_sync_boundary'}
                      else normalize(check, entry, result, root, files))
        expected=[] if empty_file_read else original['findings']
        observation.update(expected=expected, actual=actual, status='passed')
        compare(expected, actual)

    def admin_files(self, module, root):
        files = []
        for target in module.SCAN_DIRS:
            target = root / target
            paths = [target] if target.is_file() else sorted(target.rglob('*.py')) if target.is_dir() else []
            for path in paths:
                try:
                    files.append(dict(path=path.relative_to(root).as_posix(), source=path.read_text()))
                except OSError:
                    continue
        return files

    def scan(self, check, entry, module, arguments):
        if check=='hasattr_async':
            from hasattr_port import scan
            return scan(self,entry,arguments)
        if check=='no_source_inspection':
            from inspection_port import scan
            return scan(self,entry,module,arguments)
        if check=='serverplan_name_inference':
            from name_inference_port import scan
            return scan(self,entry,module,arguments)
        if check=='transaction_callback_wrappers':
            from callback_wrappers_port import scan
            return scan(self,entry,module,arguments)
        if check=='transaction_closures':
            from transaction_port import scan
            return scan(self,module,entry,arguments)
        if check=='plan_scope_ownership':
            from plan_ownership_port import scan
            return scan(self,entry,module,arguments)
        if check == 'async_sync_boundary':
            from boundary_port import scan
            return scan(self, entry, module, arguments)
        if check == 'async_patterns':
            from async_patterns_port import scan
            return scan(self, entry, module, arguments)
        if check == 'conn_or_txn_union':
            from conn_union_port import scan
            return scan(self, entry, module, arguments)
        if check == 'unresolved_forward_refs':
            from unresolved_port import scan
            return scan(self, entry, module, arguments)
        if check == 'serverplan_grammar_surface':
            from grammar_surface_port import scan
            return scan(self, entry, module, arguments)
        if check == 'serverplan_target_architecture':
            from target_architecture_port import scan
            return scan(self, module, arguments, entry=entry)
        if check == 'serverplan_state_tripwires':
            from state_tripwires_port import scan
            return scan(self,module,arguments,entry)
        if check == 'legacy_execution_surfaces':
            from legacy_execution_port import scan
            return scan(self, entry, module, arguments)
        if check == 'backend_type_strings':
            from backend_types_port import scan
            return scan(self, entry, module, arguments)
        if check == 'error_handling':
            from error_handling_port import scan
            return scan(self, entry, module, arguments)
        if check == 'no_api_root_imports':
            from api_imports_port import scan
            return scan(self, entry, module, arguments)
        if check == 'connection_caches':
            from connection_caches_port import scan
            return scan(self, entry, module, arguments)
        if check == 'backend_coupling':
            from coupling_port import scan
            return scan(self, entry, module, arguments)
        if check == 'data_layer_facade_purity':
            from facade_purity_port import scan
            return scan(self, entry, module, arguments)
        if check == 'sync_async_signatures':
            from signatures_port import scan
            return scan(self, entry, module, arguments)
        if check == 'hnsw_architecture':
            from hnsw_port import scan
            return scan(self, entry, module, arguments)
        if check == 'data_layer_compat_aliases':
            from aliases_port import scan
            return scan(self, entry, module, arguments)
        if check == 'forwardref':
            path = Path(arguments.get('filepath', arguments.get('dirpath')))
            paths = list(path.rglob('*.py')) if entry == 'lint_directory' else [path]
            files, originals, scopes = [], {}, []
            root = Path(arguments.get('root') or Path.cwd())
            for index, item in enumerate(paths):
                actual = item if item.is_absolute() else root / item
                name = f'impl/forwardref-api-{index}.py'
                scope_path = str(actual.relative_to(root)) if actual.is_relative_to(root) else str(actual)
                originals[name] = actual
                scopes.append(scope_path)
                try:
                    content = actual.read_text()
                except (OSError, UnicodeError):
                    if entry == 'lint_file_content':
                        if not actual.is_file():
                            return []
                        gate = self.native(check, 'source_gate', [], dict(callSource=arguments['source']))
                        if len(gate) > 1 or any(r['code'] != 'FWDREF_GATE' for r in gate):
                            raise AssertionError('Invalid native ForwardRef source gate')
                        if not gate:
                            return []
                        scope = self.native(check, 'file_gate', [dict(path=name, source='')],
                                            dict(logicalPaths=[name],scopePaths=[scope_path]))
                        if len(scope) > 1 or any(r['code'] != 'FWDREF_FILE_GATE' for r in scope):
                            raise AssertionError('Invalid native ForwardRef scope gate')
                        if not scope:
                            return []
                    raise
                files.append(dict(path=name, source=content))
            options = dict(callSource=arguments['source']) if entry == 'lint_file_content' else {}
            options.update(logicalPaths=list(originals),scopePaths=scopes)
            if entry == 'lint_directory':
                options.update(excludePatterns=arguments.get('exclude_patterns') or [],
                               originalPaths=[str(item) for item in paths])
            rows = self.native(check, entry, files, options)
            if any(row['code'] not in {'FWDREF001', 'FWDREF002', 'FWDREF003'} for row in rows):
                raise AssertionError('Unexpected native ForwardRef diagnostic')
            order = {f['path']: i for i, f in enumerate(files)}
            return [module.LintIssue(originals[r['path']], r['line'], r['parameters']['column'], r['message'], r['code'])
                    for r in sorted(rows, key=lambda r: (order[r['path']], r['parameters']['ordinal']))]
        if check == 'type_safety_patterns':
            from types_port import scan
            return scan(self, entry, module, arguments)
        path = Path(arguments.get('path', arguments.get('file_path', arguments.get('filepath', 'input.py'))))
        if check == 'classvar_check':
            from classvar_port import scan
            return scan(self,entry,module,arguments)
        if check == 'dynamic_module_loading':
            if entry == '_is_in_type_checking_block':
                # Preserve arbitrary list elements, including embedded line separators.
                lines, index = arguments['lines'], arguments['line_idx']
                lines[index]  # Match the source API's IndexError contract.
                rows = self.native(check, entry, [], dict(helperLines=lines, lineIndex=index))
                if len(rows) > 1 or any(r['code'] != 'DML_TYPE_CHECKING' for r in rows):
                    raise AssertionError('Invalid native TYPE_CHECKING predicate')
                return bool(rows)
            root = Path(arguments['root'])
            paths = ([path] if entry != 'find_violations' else
                     module.discover_python_files(root, arguments.get('directories'), include_tests=False))
            files, original_paths = [], {}
            for item in paths:
                name = item.relative_to(root).as_posix()
                if entry == '_scan_file_content':
                    content = arguments['content']
                else:
                    try:
                        content = item.read_text(encoding='utf-8')
                    except (OSError, UnicodeDecodeError):
                        continue
                files.append(dict(path=name, source=content))
                original_paths[name] = item
            allowed = arguments.get('allowlisted', module.ALLOWLISTED_FILES)
            rows = self.native(check, entry, files, dict(allowlistedFiles=sorted(allowed), signature=module.ALLOWLIST_SIGNATURE))
            if any(row['code'] != 'DML001' for row in rows):
                raise AssertionError('Unexpected native dynamic-loading code')
            order = {f['path']: i for i, f in enumerate(files)}
            rows.sort(key=lambda r: (order[r['path']], r['line'], r['parameters']['ordinal']))
            return [(original_paths[r['path']], r['line'], r['parameters']['snippet'], r['parameters']['label']) for r in rows]
        if check == 'async_executor':
            if entry in {'should_check_file', 'is_allowed'}:
                rows = self.native(check, entry, [], dict(path=str(arguments['filepath'])))
                code = 'EXECUTOR_STRICT' if entry == 'should_check_file' else 'EXECUTOR_ALLOWED'
                if len(rows) > 1 or any(r['code'] != code for r in rows):
                    raise AssertionError('Invalid native executor path predicate')
                return bool(rows)
            if entry == 'check_source':
                source = arguments['source']
            else:
                try:
                    source = path.read_text()
                except Exception as error:
                    print(f'Warning: Could not read {path}: {error}', file=sys.stderr)
                    return []
            rows = self.native(check, entry, [dict(path='input/source.py', source=source)],
                               dict(apiInputDirs=['input']))
            rows.sort(key=lambda r: r['line'])
            return [(r['line'], r['parameters']['source']) for r in rows]
        if check == 'admin_async_purity':
            options = dict(allowlistedFiles=list(module.ALLOWLISTED_FILES))
            if entry in {'_should_skip', '_is_allowlisted'}:
                options['path'] = arguments.get('path', arguments.get('file'))
                rows = self.native(check, entry, [], options)
                code = 'ADMIN_SKIP' if entry == '_should_skip' else 'ADMIN_ALLOWLIST'
                if len(rows) > 1 or any(r['code'] != code for r in rows):
                    raise AssertionError('Invalid native admin helper Boolean')
                return bool(rows)
            root = Path(arguments.get('project_root', arguments.get('root')))
            files = ([dict(path=path.relative_to(root).as_posix(), source=arguments['content'])]
                     if entry == '_collect_violations_from_source' else self.admin_files(module, root))
            rows = self.native(check, entry, files, options)
            order = {f['path']: index for index, f in enumerate(files)}
            rows.sort(key=lambda r: (order[r['path']], r['line'], r['parameters']['ordinal']))
            return [module.Violation(file=r['path'], line=r['line'], pattern=r['parameters']['pattern'],
                                     text=r['parameters']['text']) for r in rows]
        if check == 'no_blanket_grammar_bypass':
            from blanket_bypass_port import scan
            return scan(self,module,entry,arguments)
        if check == 'permission_error':
            from permission_port import scan_file
            return scan_file(self, module, entry, path, arguments.get('source'), arguments.get('tree'))
        if check == 'removed_compat_surfaces':
            from removed_port import scan_batch
            return scan_batch(self,module,[path])
        if check == 'versioned_lifecycle_legacy_surfaces':
            from lifecycle_port import scan_file
            return scan_file(self,module,entry,path,arguments.get('source'))

        raise AssertionError(f'No native assertion adapter: {check}:{entry}')

    def pytest_configure(self):
        self.modules = {check: importlib.import_module('scripts.lint_' + check) for check in self.checks}
        if 'serverplan_name_inference' in self.modules:
            from name_inference_port import install
            install(self,self.modules['serverplan_name_inference'])
        bind_aliases(self.modules)
        originals = codes(self.modules)
        if 'serverplan_grammar_surface' in self.modules:
            shared = importlib.import_module('scripts.lint_lib.serverplan_facts')
            originals.update(codes({'serverplan_facts': shared}))
        for check, module in self.modules.items():
            if check == 'serverplan_state_tripwires':
                from state_tripwires_port import install
                install(self,module)
            if check == 'legacy_execution_surfaces':
                from legacy_execution_port import install
                install(self, module)
            if check=='plan_composition_architecture':
                from composition_port import install
                install(self,module)
            if check=='sql_grammar_escape_hatches':
                from grammar_escape_port import install
                install(self,module)
            if check=='transaction_patterns':
                from patterns_port import install
                install(self,module)
            if check == 'no_api_root_imports':
                from api_imports_port import install
                install(self,module)
            if check == 'versioned_lifecycle_legacy_surfaces':
                from lifecycle_port import install
                install(self,module)
            if check == 'removed_compat_surfaces':
                from removed_port import install
                install(self,module)
            if check == 'permission_error':
                from permission_port import install
                install(self, module)
            if check=='no_blanket_grammar_bypass':
                from blanket_bypass_port import install
                install(self,module)
            if check=='plan_scope_ownership':
                from plan_ownership_port import install
                install(self,module)
            if check == 'async_sync_boundary':
                from boundary_port import install
                install(self, module)
            if check == 'unresolved_forward_refs':
                from unresolved_port import install
                install(self, module)
            if check == 'serverplan_grammar_surface':
                from grammar_surface_port import install
                install(self, module)
            if check == 'backend_type_strings':
                from backend_types_port import install
                install(self, module)
            if check=='no_source_inspection':
                from inspection_port import install
                install(self,module)
            if check=='hasattr_async':
                from hasattr_port import install
                install(self,module)
            if check in {'backend_type_strings', 'hasattr_async'}:
                from text_cli_port import install
                install(self, module, check)
            if check == 'classvar_check':
                from classvar_port import install
                install(self,module)
            if check == 'dynamic_module_loading':
                from dynamic_port import install
                install(self,module,detectors=True)
            if check == 'connection_caches':
                from connection_caches_port import install
                install(self, module)
            if check == 'backend_coupling':
                from coupling_port import install
                install(self, module)
            if check == 'data_layer_facade_purity':
                from facade_purity_port import DefaultTargets
                self.replace(module, 'DEFAULT_TARGETS', DefaultTargets(self, module))
            if check in {'data_layer_compat_aliases', 'data_layer_facade_purity'}:
                # Passive list storage retains the original DTO assertion without source methods.
                self.replace(module.LintResults, 'add', lambda result, violation: result.violations.append(violation))
            for entry in ENTRIES[check]:
                parts = entry.split('.')
                owner = getattr(module, parts[0]) if len(parts) == 2 else module
                original = getattr(owner, parts[-1])
                signature = inspect.signature(original)

                def wrapper(check, entry, module, original, signature):
                    @wraps(original)
                    def run(*args, **kwargs):
                        values = dict(signature.bind(*args, **kwargs).arguments)
                        result = original(*args,**kwargs) if check=='dynamic_module_loading' else self.scan(check, entry, module, values)
                        self.compare_original(check, entry, module, args, kwargs, result)
                        return result
                    if check=='backend_type_strings' and entry=='check_file' or check=='hasattr_async' and entry in {'check_file','should_exclude'}:run._rerail_native_check=check
                    return run
                self.replace(owner, parts[-1], wrapper(check, entry, module, original, signature))

        foreign_prefix=str(self.source/'scripts')+'/'
        foreign_codes={}
        def guard(frame, event, arg):
            if event != 'call':
                return
            if frame.f_code not in originals:
                code=frame.f_code
                if code not in foreign_codes:
                    name=code.co_filename.removeprefix(foreign_prefix)
                    foreign_codes[code]=(name[5:-3] if code.co_filename.startswith(foreign_prefix)
                        and name.startswith('lint_') and name.endswith('.py') and '/' not in name
                        and code.co_flags & inspect.CO_NEWLOCALS else None)
                if foreign_codes[code]:
                    self.semantic_fallbacks.append(foreign_codes[code]+':'+code.co_qualname)
                    raise AssertionError('Unselected original linter executed: '+self.semantic_fallbacks[-1])
                return
            check, name, passive = originals[frame.f_code]
            host = {'admin_async_purity': {'cli_main'},
                    'async_executor': {'cli_main'},
                    'type_safety_patterns': {'cli_main'},
                    'data_layer_compat_aliases': {'cli_main', '_print_results'},
                    'data_layer_facade_purity': {'cli_main', '_print_results'},
                    'backend_coupling': {'cli_main', 'get_project_root'},
                    'connection_caches': {'cli_main'}}
            if not passive and name not in host.get(check, set()):
                self.semantic_fallbacks.append(f'{check}:{name}')
                raise AssertionError('Original semantic execution: ' + self.semantic_fallbacks[-1])
            if not passive:
                self.host_protocol_calls.append(dict(test=self.item.nodeid if self.item else '<configuration>',check=check,entry=name))
        self.guard = guard
        sys.setprofile(guard)
        threading.setprofile(guard)

    def pattern_search(self, pattern, source):
        if self.item.path.name == 'test_lint_connection_caches.py':
            module = self.modules['connection_caches']
            indices = [i for i, (p, _) in enumerate(module.ALL_PATTERNS) if p is pattern]
            check, code = 'connection_caches', 'CC_PATTERN'
        elif self.item.path.name == 'test_lint_type_safety_patterns.py':
            module = self.modules['type_safety_patterns']
            names = ['STR_EXPR_PATTERN', 'STR_FIELD_PATTERN', 'EXPR_ANY_ANY_PATTERN', 'DEPRECATED_OP_PATTERN', 'DEPRECATED_FILTER_METHOD_PATTERN']
            indices = [i for i, name in enumerate(names) if getattr(module, name) is pattern]
            check, code = 'type_safety_patterns', 'TSP_PATTERN'
        elif self.item.path.name == 'test_lint_dynamic_module_loading.py':
            module = self.modules['dynamic_module_loading']
            indices = [i for i, (p, _) in enumerate([*module._PATTERNS, *module._IMPORT_MODULE_PATTERNS]) if p is pattern]
            check, code = 'dynamic_module_loading', 'DML_PATTERN'
        elif self.item.path.name == 'test_lint_async_executor.py':
            module = self.modules['async_executor']
            indices = [i for i, p in enumerate([module.EXECUTOR_PATTERN, module.NOQA_PATTERN]) if p is pattern]
            check, code = 'async_executor', 'EXECUTOR_PATTERN'
        elif self.item.path.name == 'test_lint_hasattr_async.py':
            module = self.modules['hasattr_async']
            indices = [kind for kind in ['hasattr', 'getattr'] if getattr(module, kind.upper() + '_ASYNC_PATTERN') is pattern]
            check, code = 'hasattr_async', 'ASYNC_PATTERN'
        elif self.item.path.name == 'test_lint_no_source_inspection.py':
            module = self.modules['no_source_inspection']
            indices = [i for i, (p, _) in enumerate(module.FORBIDDEN_PATTERNS) if p is pattern]
            check, code = 'no_source_inspection', 'NSI_PATTERN'
        else:
            module = self.modules['admin_async_purity']
            indices = [i for i, (p, _) in enumerate(module.FORBIDDEN_PATTERNS) if p is pattern]
            check, code = 'admin_async_purity', 'ADMIN_PATTERN'
            if not indices:
                indices = [i for i, p in enumerate(module.SKIP_PATTERNS) if p is pattern]
                code = 'ADMIN_SKIP_PATTERN'
        if len(indices) != 1:
            raise AssertionError('Original test supplied an unknown regex constant')
        entry = 'skip_pattern_search' if code == 'ADMIN_SKIP_PATTERN' else 'pattern_search'
        rows = self.native(check, entry, [dict(path='input.py', source='')],
                           dict(patternOrdinal=indices[0], patternText=source))
        if len(rows) > 1 or any(r['code'] != code for r in rows):
            raise AssertionError('Native regex Boolean returned invalid rows')
        if not rows:
            return None
        if check == 'hasattr_async':
            method = rows[0]['parameters']['method']
            return NativeRegexMatch({1: method})
        return NativeRegexMatch({})

    def anchored_pattern_match(self, pattern, source):
        module = self.modules['connection_caches']
        if pattern is not module.MODULE_BRANCH_REVISION_DICT or not pattern.pattern.startswith('^'):
            raise AssertionError('Only the source-anchored cache regex has an original match assertion')
        return self.pattern_search(pattern, source)

    def allowed_pattern_witness(self, pattern):
        # These two source parameters skip their assertion because the test cannot
        # fabricate a richer-regex witness. The original pattern matches these tokens.
        witnesses = {r'''getattr\(scheme, ['\"]resolve_context_async''': "getattr(scheme, 'resolve_context_async",
                     r'''getattr\(graph_scheme, ['\"]resolve_context_async''': "getattr(graph_scheme, 'resolve_context_async"}
        if pattern not in witnesses:
            raise AssertionError('A source-evaluated witness is required for this original skip')
        return witnesses[pattern]

    def pytest_collection_modifyitems(self, items):
        self.collected = [item.nodeid for item in items]
        items[:] = items[self.part::self.parts]
        for item in items:
            if item.path.name not in {'test_lint_connection_caches.py', 'test_lint_no_source_inspection.py', 'test_lint_hasattr_async.py', 'test_lint_admin_async_purity.py', 'test_lint_async_executor.py', 'test_lint_dynamic_module_loading.py', 'test_lint_type_safety_patterns.py', 'test_lint_serverplan_state_tripwires.py'}:
                continue
            source_lines, source_start = inspect.getsourcelines(item.obj)
            original = textwrap.dedent(''.join(source_lines))
            tree = ast.parse(original)
            function = next(n for n in tree.body if isinstance(n, ast.FunctionDef))
            function.decorator_list = []
            changed = []
            operations = []
            if item.path.name=='test_lint_serverplan_state_tripwires.py':
                from state_assertion_translation import translate
                operations=translate(tree)
                changed.extend(op['line'] for op in operations)

            class Searches(ast.NodeTransformer):
                def visit_Attribute(self, node):
                    if (item.path.name == 'test_lint_type_safety_patterns.py' and node.attr == 'Match'
                            and isinstance(node.value, ast.Name) and node.value.id == '_re'):
                        changed.append(node.lineno)
                        operations.append(dict(line=node.lineno, kind='regex-match-type'))
                        return ast.copy_location(ast.Name(id='_rr_RegexMatch', ctx=ast.Load()), node)
                    return self.generic_visit(node)

                def visit_Call(self, node):
                    self.generic_visit(node)
                    if (item.path.name == 'test_lint_connection_caches.py' and isinstance(node.func, ast.Attribute)
                            and node.func.attr == 'match'):
                        if len(node.args) != 1 or node.keywords:
                            raise AssertionError('Unsupported original anchored regex assertion')
                        changed.append(node.lineno)
                        operations.append(dict(line=node.lineno, kind='anchored-regex-match'))
                        return ast.copy_location(ast.Call(func=ast.Name(id='_rr_anchored_match', ctx=ast.Load()),
                            args=[node.func.value, *node.args], keywords=[]), node)
                    if isinstance(node.func, ast.Attribute) and node.func.attr == 'search':
                        if len(node.args) != 1 or node.keywords:
                            raise AssertionError('Unsupported original regex assertion')
                        changed.append(node.lineno)
                        operations.append(dict(line=node.lineno, kind='regex-search'))
                        return ast.copy_location(ast.Call(func=ast.Name(id='_rr_pattern_search', ctx=ast.Load()),
                            args=[node.func.value, *node.args], keywords=[]), node)
                    return node

                def visit_Expr(self, node):
                    if (isinstance(node.value, ast.Call) and isinstance(node.value.func, ast.Attribute)
                            and isinstance(node.value.func.value, ast.Name) and node.value.func.value.id == 'pytest'
                            and node.value.func.attr == 'skip' and item.path.name == 'test_lint_hasattr_async.py'):
                        changed.append(node.lineno)
                        operations.append(dict(line=node.lineno, kind='reviewed-source-skip'))
                        if function.name == 'test_hasattr_regex_does_not_match_non_async':
                            # The original regex rejects _async_helper; the skip's explanation
                            # is wrong. Execute the original negative assertion through native.
                            return ast.copy_location(ast.Pass(), node)
                        if function.name == 'test_every_allowed_async_pattern_makes_line_allowed':
                            return ast.copy_location(ast.Assign(targets=[ast.Name(id='token', ctx=ast.Store())],
                                value=ast.Call(func=ast.Name(id='_rr_allowed_pattern_witness', ctx=ast.Load()),
                                               args=[ast.Name(id='pattern', ctx=ast.Load())], keywords=[])), node)
                        raise AssertionError('An unreviewed original skip cannot be removed')
                    return self.generic_visit(node)

            tree = Searches().visit(tree)
            if not changed:
                continue
            ast.fix_missing_locations(tree)
            ast.increment_lineno(tree, source_start - 1)
            namespace = {**item.obj.__globals__, '_rr_pattern_search': self.pattern_search,
                         '_rr_anchored_match': self.anchored_pattern_match,
                         '_rr_allowed_pattern_witness': self.allowed_pattern_witness, '_rr_RegexMatch': NativeRegexMatch}
            if item.path.name=='test_lint_serverplan_state_tripwires.py':
                from state_tripwires_port import analysis_contexts
                namespace['_rr_state_analysis_scope']=lambda path,source,family: analysis_contexts(
                    self,self.modules['serverplan_state_tripwires'],path,source)[family]
            exec(compile(tree, str(item.path), 'exec'), namespace)
            item.obj = namespace[function.name]
            self.translations.append(dict(test=item.nodeid, sourceSha256=sha(original.encode()),
                                          translatedSha256=sha(ast.unparse(tree).encode()),
                                          sites=[source_start + line - 1 for line in changed],
                                          operations=[dict(site=source_start + op['line'] - 1, kind=op['kind']) for op in operations],
                                          operation=('Native analysis-scope observations; original assertion predicates retained'
                                              if item.path.name=='test_lint_serverplan_state_tripwires.py' else
                                              'Native regex searches and match-type counterparts; original assertion predicates retained')))

    @pytest.hookimpl(tryfirst=True)
    def pytest_runtest_setup(self, item):
        self.item = item
        sys.setprofile(self.guard)
        threading.setprofile(self.guard)

    def pytest_runtest_logreport(self, report):
        self.cases.setdefault(report.nodeid, []).append(dict(phase=report.when, outcome=report.outcome))
        if report.when == 'teardown':
            self.item = None

    def pytest_unconfigure(self):
        sys.setprofile(None)
        threading.setprofile(None)
        for module, name, original in reversed(self.originals):
            setattr(module, name, original)
        self.worker.stdin.close()
        try:
            self.worker.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.worker.kill()
            self.worker.wait()
        self.cache.cleanup()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--checks', required=True)
    parser.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot')
    parser.add_argument('--tests', nargs='+', action='extend', help='Explicit unchanged modules or pytest node IDs; repeated lists accumulate; defaults to each check\'s direct module')
    parser.add_argument('--rule-root', type=Path, default=ROOT / 'gate/data-layer/packs/data-layer/candidates/rr',
                        help='Rule directory to execute; mutation tests supply a directory of disabled policies')
    parser.add_argument('--part', type=int, default=0)
    parser.add_argument('--parts', type=int, default=1)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--baseline', type=Path,
                        default=fixtures_path('host', 'data-layer-port', 'batch-01-recording.json'))
    args = parser.parse_args()
    source, engine, out = args.source.resolve(), args.engine.resolve(), args.out.resolve()
    # Bind executed code, including imported legacy DTO adapters. An unrelated inventory
    # tool must not invalidate an assertion run merely because it shares this directory.
    harness_root = ROOT / 'scripts/dsl'
    harness_before = {p.resolve(): sha(p.read_bytes()) for p in harness_root.rglob('*.py')}
    if git(source, 'rev-parse', 'HEAD') != PIN or not 0 <= args.part < args.parts:
        parser.error('Wrong source pin or shard')
    checks = args.checks.split(',')
    if any(check not in ENTRIES for check in checks):
        parser.error('A full assertion adapter is required for each selected check')
    targets = args.tests or [f'scripts/tests/test_lint_{check}.py' for check in checks]
    tests = [str(source / target) for target in targets]
    test_files = [source / target.split('::', 1)[0] for target in targets]
    source_hashes = {str(p.relative_to(source)): sha(p.read_bytes()) for p in [*test_files, *[source / f'scripts/lint_{c}.py' for c in checks]]}
    digest = sha(engine.read_bytes())
    sys.path[:0] = [str(source), str(source / 'scripts')]
    if 'hasattr_async' in checks:
        module = importlib.import_module('scripts.lint_hasattr_async')
        if module.HASATTR_ASYNC_PATTERN.search('hasattr(txn, "_some_async_helper")') is not None:
            raise AssertionError('The source regex changed; re-audit the skipped negative assertion')
        import re
        for pattern, witness in [(r'''getattr\(scheme, ['\"]resolve_context_async''', "getattr(scheme, 'resolve_context_async"),
                                 (r'''getattr\(graph_scheme, ['\"]resolve_context_async''', "getattr(graph_scheme, 'resolve_context_async")]:
            if pattern not in module.ALLOWED_PATTERNS or re.search(pattern, f'    if hasattr(txn, "{witness}"):') is None:
                raise AssertionError('The source no longer validates this skipped-case witness')
    backend = Suite(source, engine, checks, args.part, args.parts, args.baseline.resolve(), args.rule_root.resolve(),execution_mode=args.mode)
    from framework_fixtures import prepare
    prepare(backend, targets)
    os.chdir(source)
    code = pytest.main(['--rootdir', str(source), '-c', str(source / 'pyproject.toml'), '-q', *map(str, tests)], plugins=[backend])
    from data_layer_probe_record import verify_sources
    source_checked = verify_sources(source, source, PIN)
    harness_paths = {Path(__file__).resolve(), Path(__file__).with_name('replay.py').resolve()}
    for module in tuple(sys.modules.values()):
        filename = getattr(module, '__file__', None)
        if filename and Path(filename).suffix == '.py':
            path = Path(filename).resolve()
            if path.is_relative_to(harness_root):
                harness_paths.add(path)
    harness_hashes = {str(p.relative_to(ROOT)): harness_before.get(p) for p in sorted(harness_paths)}
    wanted = Counter((r['test'], r['check'], r['entry']) for r in backend.baseline['events'] if r['test'] in backend.cases and r['check'] in checks)
    incomplete = any({p['phase']: p['outcome'] for p in phases} != dict(setup='passed', call='passed', teardown='passed')
                     for phases in backend.cases.values())
    moving_harness = any(digest is None or sha((ROOT / name).read_bytes()) != digest for name, digest in harness_hashes.items())
    if sha(engine.read_bytes()) != digest or backend.semantic_fallbacks or incomplete or backend.observed != wanted or moving_harness:
        code = 1
    write_json(out, dict(format='rerail.rr-original-assertions/v1', sourcePin=PIN, oracle=False,
        engineSha256=digest, exitCode=int(code), sourceFiles=source_hashes,
        ruleRoot=str(args.rule_root.resolve()),
        testTargets=targets,
        api=args.mode+'-cli', mode=args.mode, harnessFiles=harness_hashes,
        policySha256={check:sha((args.rule_root/(check.replace('_','-')+'.rr')).read_bytes()) for check in checks},
        sourceFilesChecked=source_checked, missingResults=[dict(test=k[0], check=k[1], entry=k[2], count=n-backend.observed[k])
            for k, n in wanted.items() if n != backend.observed[k]],
        collected=backend.collected, part=args.part, parts=args.parts, cases=backend.cases,
        calls=backend.calls, assertionTranslations=backend.translations,
        configurationBindings=getattr(backend, 'configuration_bindings', {}),
        hostTransfers=getattr(backend, 'host_transfers', []),
        originalRecordingSha256=backend.baseline_sha, detectorObservations=backend.observations,
        pythonHashSeed=os.environ.get('PYTHONHASHSEED'),
        executionIsolation='One owned persistent cache per assertion shard; shared-cache concurrency is tested separately',
        semanticFallbacks=backend.semantic_fallbacks,
        hostProtocolCalls=backend.host_protocol_calls,
        casesWithoutNativeCalls=[case for case in backend.cases if not any(c['test'] == case for c in backend.calls)]))
    raise SystemExit(code)


if __name__ == '__main__':
    main()
