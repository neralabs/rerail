# SPDX-License-Identifier: Apache-2.0
"""Marshal original scanner API arguments and DTOs around native execution."""
from __future__ import annotations
import ast
from collections import Counter
from dataclasses import asdict, fields
from functools import wraps
import inspect
import json
from pathlib import Path

from data_layer_batch_eight import tree_source


def install(backend):
    entries = {
        'eager_resolve': ['_scan_source', '_scan_file', '_scan_preprocess_ast', '_scan_lower_compile_ast',
                          '_scan_transitive_eager_ast', '_scan_legacy_boundary_definition_ast', '_scan_external_io_boundary_definition_ast'],
        'lazy_architecture': ['scan_files', '_scan_analyzed_files'],
        'pure_lazy_runtime_purity': ['scan_source', 'scan_paths', 'scan_paths_cached'],
        'cached_pure_lazy_allowlist': ['policy_violations', 'memoized_policy_violations'],
        'plan_helpers_allowlist': ['scan_file_content', 'scan_file', 'scan_repo', '_violations_for_files'],
        'legacy_callback_quarantine': ['scan_file_content', 'scan_repo'],
        'lazy_sql_surface': ['scan_source', 'scan_file', 'scan_repo'],
        'readthrough_authority_architecture': ['scan_sources'],
        'authentication_plan_purity': ['scan_sources'],
        'lazy_adapter_ownership': ['scan_source', 'violations'],
    }
    backend.fact_sources = {}

    def wrap(check, entry, original, module):
        signature = inspect.signature(original)
        @wraps(original)
        def execute(*args, **kwargs):
            arguments = dict(signature.bind_partial(*args, **kwargs).arguments)
            if check=='cached_pure_lazy_allowlist':
                functions=arguments.get('functions',arguments.get('discovered',()))
                entries=arguments.get('entries',arguments.get('policy_entries',()))
                records=[]
                for function in functions:
                    value=asdict(function);value['path']=str(value['path']);records.append(value)
                kind='memoized' if entry=='memoized_policy_violations' else 'cached'
                rows=backend.execute(check,entry,[],contract=kind+'-policy-comparison',parameters=dict(functions=records,entries=[asdict(entry) for entry in entries]))
                return tuple(row['code'] for row in rows)
            if check=='lazy_architecture' and entry=='_scan_analyzed_files':
                files=[];original_paths={}
                for file,analyzed in arguments['by_file'].items():
                    name=logical(file,module.ROOT);original_paths[name]=file
                    functions=[]
                    for fact in analyzed.facts:
                        value=asdict(fact);value['file']=logical(value['file'],module.ROOT);functions.append(value)
                    files.append(dict(path=name,module=analyzed.module_name,functions=functions))
                native=backend.execute(check,entry,[],contract='lazy-analyzed',parameters=dict(files=files))
                result=list(arguments.get('extra_violations',[]))
                for row in native:
                    value=json.loads(row['variables']['row']['payload']);value['file']=original_paths[value['file']]
                    result.append(module.Violation(**value))
                counts=Counter(row.rule for row in result)
                return {rule:counts[rule] for rule in module.RULES},result
            root = Path(arguments.get('root', getattr(module, 'ROOT', getattr(module, 'REPO_ROOT', backend.root))))
            path = arguments.get('path', arguments.get('file_path', arguments.get('rel_path')))
            source = arguments.get('source', arguments.get('content'))
            tree = next((value for value in arguments.values() if isinstance(value, ast.AST)), None)
            if tree is not None:
                source = ast.unparse(tree) if check == 'eager_resolve' else tree_source(tree, backend.trees)
                if source is None: raise AssertionError('Native adapter has no original AST source')
            options = {}
            if check == 'eager_resolve': options['externalIoBoundaries'] = module._load_external_io_boundary_allowlist()
            paths = arguments.get('paths', arguments.get('files'))
            if 'by_file' in arguments:
                paths = list(arguments['by_file'])
            if check == 'cached_pure_lazy_allowlist':
                functions = arguments.get('functions', arguments.get('discovered', ()))
                paths = sorted({item.path for item in functions})
                options['policyEntries'] = json.dumps([asdict(item) for item in arguments.get('entries', arguments.get('policy_entries', ()))])
            if paths is None:
                if path is not None: paths = [path]
                elif 'root' in arguments: paths = sorted(root.rglob('*.py'))
                elif 'roots' in arguments: paths = [p for directory in arguments['roots'] for p in Path(directory).rglob('*.py')]
                else: raise AssertionError(f'Unimplemented native input adapter: {check}.{entry}')
            paths = list(paths)
            if check == 'pure_lazy_runtime_purity' and entry in ('scan_paths', 'scan_paths_cached'):
                options['primaryPaths'] = [logical(p, root) for p in paths]
                if arguments.get('directories') is not None:
                    paths = sorted(set(paths) | {p for directory in arguments['directories'] for p in (root/directory).rglob('*.py') if 'tests' not in p.parts})
            overrides = arguments.get('source_overrides', {}) or {}
            files, originals, raw_files = [], {}, []
            distinct_raw=False
            for current in paths:
                current = Path(current)
                raw_text=None
                if check=='lazy_architecture' and module._parse_file is not module._FRAMEWORK_ONLY_PARSE_FILE:
                    parsed=module._parse_file(current)
                    if parsed is None:continue
                    source_tree=tree_source(parsed,backend.trees)
                    if source_tree is None:raise AssertionError('Missing original AST source')
                    try:raw_text=current.read_text()
                    except OSError:raw_text=ast.unparse(parsed)
                    source=source_tree;path=current
                if source is not None and path is not None and current == Path(path): text = source
                elif current in overrides: text = overrides[current]
                elif current in backend.fact_sources: text = backend.fact_sources[current]
                elif check == 'lazy_architecture' and module._parse_file is not module._FRAMEWORK_ONLY_PARSE_FILE:
                    parsed = module._parse_file(current)
                    if parsed is None: continue
                    text = tree_source(parsed, backend.trees)
                    if text is None: raise AssertionError(f'No source for mocked AST: {current}')
                else:
                    try: text = current.read_text(encoding='utf-8')
                    except (OSError, UnicodeError): continue
                name = logical(current, root)
                files.append(dict(path=name, source=text)); originals[name] = current
                raw_files.append(dict(path=name,source=text if raw_text is None else raw_text))
                distinct_raw |= raw_text is not None and raw_text!=text
            native_entry = 'analyzed-functions' if check=='lazy_architecture' and entry=='_scan_analyzed_files' else 'scan_file' if entry == '_violations_for_files' else entry
            if check=='lazy_architecture' and distinct_raw:
                core=backend.execute(check,'scan_files.ast-component',files,options)
                raw=backend.execute(check,'_scan_deleted_cold_bootstrap_surfaces',raw_files,options)
                # Preserve the linter's collector, raw-text, then function phases.
                extras=[row for row in core if row['variables']['row']['producer_phase']==0]
                functions=[row for row in core if row['variables']['row']['producer_phase']!=0]
                result=[dto(check,module,row,files if row in core else raw_files,originals,root) for row in [*extras,*raw,*functions]]
                counts=Counter(row.rule for row in result)
                return {rule:counts[rule] for rule in module.RULES},result
            findings = backend.execute(check, native_entry, files, options)
            if check == 'cached_pure_lazy_allowlist': return tuple(sorted(finding['code'] for finding in findings))
            result = [dto(check, module, finding, files, originals, root) for finding in findings]
            if check == 'lazy_architecture':
                result.extend(arguments.get('extra_violations', []))
                counts = Counter(finding.rule for finding in result)
                return {rule:counts[rule] for rule in module.RULES}, result
            return result
        return execute

    for check, names in entries.items():
        module = backend.modules[check]
        for name in names:
            backend.replace(module, name, wrap(check, name, getattr(module, name), module))
    from native_cached import install as cached
    cached(backend)
    from native_helpers import install as helpers
    helpers(backend)
    install_plan_helpers(backend)
    from native_lazy import install as lazy
    lazy(backend)


def install_plan_helpers(backend):
    module = backend.modules['plan_helpers_allowlist']
    def metadata(path, detector=None, expression=None):
        parameters = dict(path=str(path), classes=[], functions=[], imports={}, module_aliases=[])
        if detector is not None:
            parameters.update(classes=detector.class_stack, functions=detector.function_stack,
                              imports=detector.imported_names, module_aliases=sorted(detector.plan_helper_module_aliases))
        if expression is not None:
            parameters.update(kind={'Name':'identifier','Attribute':'attribute'}.get(type(expression).__name__,type(expression).__name__),
                              name=getattr(expression,'id',getattr(expression,'attr',None)),
                              receiver=getattr(getattr(expression,'value',None),'id',None))
        rows = backend.execute('plan_helpers_allowlist', 'metadata', [], contract='plan-metadata', parameters=parameters)
        if len(rows) != 1: raise AssertionError('Native plan metadata did not return exactly one row')
        return rows[0]['variables']['row']
    backend.replace(module, '_should_scan', lambda path:bool(metadata(path)['should_scan']))
    cls = module.PlanHelperDetector
    backend.replace(cls, '_path_to_module', staticmethod(lambda path:metadata(path)['module']))
    backend.replace(cls, '_context', lambda self:metadata(self.path,self)['context'])
    backend.replace(cls, '_callsite', lambda self:metadata(self.path,self)['callsite'])
    backend.replace(cls, '_is_allowlisted', lambda self:bool(metadata(self.path,self)['allowlisted']))
    backend.replace(cls, '_helper_name', lambda self,node:metadata(self.path,self,node.func)['helper'])
    backend.replace(cls, '_helper_name_from_expr', lambda self,node:metadata(self.path,self,node)['helper'])


def logical(path, root):
    path = Path(path)
    if path.is_absolute():
        if path.is_relative_to(root): return path.relative_to(root).as_posix()
        # Preserve package markers for production path predicates and module names.
        parts = path.parts
        start = next((i for i, part in enumerate(parts) if part in ('api', 'impl', '_internal', 'data_layer') or len(part) > 5 and part[0] == 'l' and part[1:5].isdigit() and part[5] == '_'), None)
        return Path(*parts[start:]).as_posix() if start is not None else path.name
    return path.as_posix()


def dto(check, module, finding, files, originals, root):
    row = finding.get('variables', {}).get('row', {})
    line = finding['span']['startLine'] + 1
    path = originals.get(finding['path'], root/finding['path'])
    source = next((file['source'] for file in files if file['path'] == finding['path']), '')
    lines = source.splitlines(); text = lines[line-1].strip() if 0 < line <= len(lines) else ''
    cls = getattr(module, 'Violation', getattr(module, 'Finding', getattr(module, 'StructuredViolation', None)))
    message = row.get('message', finding['message'])
    values = dict(code=finding['code'], rule=finding['code'], path=path, file=path, file_path=path,
                  line=line, line_num=line, line_number=line, message=message,
                  reason=finding['code'], text=row.get('text', text), pattern_name=finding['code'],
                  symbol=row.get('symbol', source.encode('utf-8')[finding['span']['startByte']:finding['span']['endByte']].decode('utf-8') if 'SURFACE' in finding.get('captures',{}) else ''), qualname=row.get('qualname', ''), callsite=row.get('callsite', ''),
                  linter_name=check, severity='error', stable_scope=row.get('stable_scope'))
    if check == 'plan_helpers_allowlist': values.update(file=Path(finding['path']), line=text, reason=finding['code'])
    if check == 'legacy_callback_quarantine': values['file_path'] = finding['path']
    return cls(**{field.name:values[field.name] for field in fields(cls) if field.name in values})
