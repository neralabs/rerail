"""Run source-derived contracts through the same native API adapters as the original assertions.

These contracts supplement the unchanged upstream suites. They never count as original pytest cases.
"""
import argparse
import contextlib
import importlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from types import SimpleNamespace

from dto import canonical
from migration import ROOT, PIN, git, sha, compare, read_json, write_json


def selected_cases(cases, part, parts):
    """Partition by corpus position while retaining indices into the complete source oracle."""
    if not 0 <= part < parts <= len(cases):
        raise ValueError('Invalid fresh-contract partition')
    return [(index, case) for index, case in enumerate(cases) if index % parts == part]


def arguments(values, root):
    def convert(value):
        if isinstance(value,dict) and set(value)=={'tuple'}:
            return tuple(convert(item) for item in value['tuple'])
        if isinstance(value,dict) and set(value)=={'frozenset'}:
            return frozenset(convert(item) for item in value['frozenset'])
        if isinstance(value, dict) and set(value) == {'pythonAst', 'mode'}:
            import ast
            tree = ast.parse(value['pythonAst'], mode=value['mode'])
            return tree.body if value['mode'] == 'eval' else tree
        if isinstance(value, dict) and set(value) == {'path', 'relative'} and value['relative'] is True:
            return Path(value['path'])
        if isinstance(value, dict) and set(value) == {'path'} and isinstance(value['path'], str):
            return root / value['path']
        if isinstance(value, list):
            return [convert(item) for item in value]
        if isinstance(value, dict):
            return {key: convert(item) for key, item in value.items()}
        return value
    return convert(values)


def invoke(call, root):
    try:
        return dict(result=canonical(call(), root))
    except Exception as error:
        if isinstance(error,SyntaxError):
            return dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=canonical(error.args,root),
                attributes={key:canonical(getattr(error,key),root) for key in ['filename','lineno','offset','end_lineno','end_offset','text','msg','print_file_and_line']}))
        if isinstance(error, OSError):
            return dict(error=dict(type=type(error).__module__ + '.' + type(error).__qualname__,
                args=canonical(error.args, root), filename=canonical(error.filename, root), errno=error.errno))
        if isinstance(error, UnicodeError):
            return dict(error=dict(type=type(error).__module__ + '.' + type(error).__qualname__,
                                   args=canonical(error.args, root)))
        if isinstance(error, (ValueError, KeyError)):
            return dict(error=dict(type=type(error).__module__ + '.' + type(error).__qualname__,
                                   args=canonical(error.args, root)))
        raise


def run(args):
    args.out = args.out.resolve()
    source, corpus = args.source.resolve(), read_json(args.cases)
    if git(source, 'rev-parse', 'HEAD') != PIN:
        raise ValueError('Wrong source pin')
    checks = sorted({case['check'] for case in corpus['cases']})
    sys.path[:0] = [str(source), str(source / 'scripts')]
    modules = {check: importlib.import_module('scripts.lint_' + check) for check in checks}
    source_files = {}
    for check in checks:
        name = 'scripts/lint_' + check + '.py'
        raw = (source / name).read_bytes()
        if raw != subprocess.check_output(['git', '-C', str(source), 'show', PIN + ':' + name]):
            raise ValueError('Changed original module: ' + name)
        source_files[name] = sha(raw)
    native = args.command == 'native'
    if not native and (args.part, args.parts) != (0, 1):
        raise ValueError('Record the complete offline source oracle before native sharding')
    selected = selected_cases(corpus['cases'], args.part, args.parts)
    recorded = read_json(args.oracle) if native else None
    if recorded is not None and (recorded['sourcePin'] != PIN or recorded['casesSha256'] != sha(args.cases.read_bytes()) or recorded['sourceFiles'] != source_files):
        raise ValueError('Fresh oracle belongs to different source or cases')
    if recorded is not None and (recorded.get('execution') != 'offline-source' or
            len(recorded['results']) != len(corpus['cases']) or
            any(row['status'] != 'passed' for row in recorded['results'])):
        raise ValueError('Fresh oracle is incomplete or is not an offline source recording')
    if len({case['id'] for case in corpus['cases']}) != len(corpus['cases']):
        raise ValueError('Duplicate fresh case identities')
    report = dict(format='rerail.rr-fresh-contract/v1', fullMigrationAcceptance=False, sourcePin=PIN, originalPytestCases=0,
        execution='native-snapshot' if native else 'offline-source', sourceFiles=source_files,
        casesSha256=sha(args.cases.read_bytes()), results=[], semanticFallbacks=[],
        part=args.part, parts=args.parts, totalCases=len(corpus['cases']),
        selectedCaseIndices=[index for index, _ in selected])
    backend = None
    harness_paths = [Path(__file__), *[Path(__file__).with_name(p) for p in ['suite.py', 'replay.py', 'dto.py', 'migration.py']],
        ROOT / 'scripts/dsl/data_layer_audit/native_guard.py']
    report['harnessFiles'] = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in harness_paths}
    harness_before = {p.resolve(): sha(p.read_bytes()) for p in (ROOT / 'scripts/dsl').rglob('*.py')}
    if native:
        from suite import Suite
        from native_guard import codes
        engine = args.engine.resolve()
        report['engineSha256'] = sha(engine.read_bytes())
        report['oracleSha256'] = sha(args.oracle.read_bytes())
        rule_root=getattr(args,'rule_root',None)
        backend = Suite(source, engine, checks, 0, 1, args.baseline.resolve(), rule_root.resolve() if rule_root is not None else None)
        original_codes = codes(modules)
        if 'async_sync_delegation' in modules:
            from async_sync_delegation_port import install
            install(backend,modules['async_sync_delegation'])
        if 'grammar_allowlist_burndown' in modules:
            from grammar_allowlist_burndown_port import install
            install(backend,modules['grammar_allowlist_burndown'])
        if 'grammar_fragment_usage' in modules:
            from grammar_fragment_usage_port import install
            install(backend,modules['grammar_fragment_usage'])
        if 'grammar_callable_constructors' in modules:
            from grammar_callable_constructors_port import install
            install(backend,modules['grammar_callable_constructors'])
        if 'grammar_str_params' in modules:
            from grammar_str_params_port import install
            install(backend,modules['grammar_str_params'])
        if 'grammar_render_methods' in modules:
            from grammar_render_methods_port import install
            install(backend,modules['grammar_render_methods'])
        if 'sql_grammar_escape_hatches' in modules:
            from grammar_escape_port import install
            install(backend,modules['sql_grammar_escape_hatches'])
        if 'transaction_patterns' in modules:
            from patterns_port import install
            install(backend, modules['transaction_patterns'])
        if 'lazy_result_usage' in modules:
            from lazy_results_port import install
            install(backend, modules['lazy_result_usage'])
        if 'regex_sql_parsing' in modules:
            from regex_sql_port import install
            install(backend,modules['regex_sql_parsing'])
        if 'render_raw_strings' in modules:
            from render_raw_strings_port import install
            install(backend, modules['render_raw_strings'])
        if 'render_fstrings' in modules:
            from render_fstrings_port import install
            install(backend, modules['render_fstrings'])
        if 'module_level_caches' in modules:
            from module_caches_port import install
            install(backend, modules['module_level_caches'])
        def guard(frame, event, arg):
            if event == 'call' and frame.f_code in original_codes:
                check, name, passive = original_codes[frame.f_code]
                if check in {'grammar_str_params','grammar_callable_constructors'} and name=='cli_main':
                    report.setdefault('hostProtocolCalls',[]).append(dict(test=backend.item.nodeid,entry=name,code=frame.f_code.co_name))
                    return
                if not passive:
                    report['semanticFallbacks'].append(check + ':' + name)
                    raise AssertionError('Original detector invoked: ' + check + ':' + name)
        sys.setprofile(guard)
        threading.setprofile(guard)
    previous_directory = Path.cwd()
    os.chdir(source)
    try:
        for index, case in selected:
            with tempfile.TemporaryDirectory(prefix='rerail-fresh-contract-') as temporary:
                root = Path(temporary)
                for file in case['files']:
                    path = root / file['path']
                    if not path.is_relative_to(root) or '..' in Path(file['path']).parts:
                        raise ValueError('Invalid fixture path')
                    path.parent.mkdir(parents=True, exist_ok=True)
                    if 'bytesHex' in file:
                        path.write_bytes(bytes.fromhex(file['bytesHex']))
                    else:
                        path.write_text(file['source'])
                os.chdir(root if case.get('cwd') == 'fixture' else source)
                values = arguments(case['arguments'], root)
                module = modules[case['check']]
                if case['check']=='async_sync_delegation':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='body_identity':
                        from async_sync_delegation_port import body_identity_probe
                        call=lambda:body_identity_probe(module,values['tree'])
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='grammar_allowlist_burndown':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='cli':
                        from grammar_allowlist_burndown_port import cli_probe
                        call=lambda:cli_probe(module,values)
                    elif case['entry']=='reporter_facts':
                        from grammar_allowlist_burndown_port import reporter_probe
                        call=lambda:reporter_probe(module,values)
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='grammar_callable_constructors':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='cli':
                        from grammar_callable_constructors_port import cli_probe
                        call=lambda:cli_probe(module,values)
                    elif case['entry']=='format':
                        from grammar_callable_constructors_port import format_probe
                        call=lambda:format_probe(module,values)
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='grammar_str_params':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='cli':
                        from grammar_str_params_port import cli_probe
                        call=lambda:cli_probe(module,values)
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='grammar_render_methods':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='body_source':
                        call=lambda:module.RenderBodyAnalyzer('x.py',[],'Fragment')._get_source(**values)
                    elif case['entry']=='configured_scan':
                        from grammar_render_methods_port import configured_scan
                        call=lambda:configured_scan(module,values)
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='regex_sql_parsing':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='checker':call=lambda:module.RegexSqlParsingChecker().check_file(**values)
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='render_raw_strings':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='format_supplied':
                        def call():
                            reports=[module.FileReport(path=v['path'],render_methods_scanned=v['methods'],violations=[module.RawStringViolation(**row) for row in v['violations']]) for v in values['reports']]
                            return getattr(module,'format_'+values['mode'])(reports,values['root'])
                    elif case['entry']=='formats':
                        def call():
                            reports=module.collect_reports(**values)
                            return [module.format_summary(reports,values['root']),module.format_detailed(reports,values['root'])]
                    elif case['entry']=='collector':
                        from render_raw_strings_port import collector_result
                        call=lambda:collector_result(module,values,backend if native else None)
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='render_fstrings':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='format_supplied':
                        def call():
                            reports=[module.FileReport(path=v['path'],render_methods_scanned=v['methods'],violations=[module.FStringViolation(**row) for row in v['violations']]) for v in values['reports']]
                            return getattr(module,'format_'+values['mode'])(reports,values['root'])
                    elif case['entry']=='formats':
                        def call():
                            reports=module.collect_reports(**values)
                            return [module.format_summary(reports,values['root']),module.format_detailed(reports,values['root'])]
                    elif case['entry']=='collector':
                        from render_fstrings_port import collector_result
                        call=lambda:collector_result(module,values,backend if native else None)
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='module_level_caches':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='collector':
                        from module_caches_port import collector_result
                        call=lambda:collector_result(module,values,backend if native else None)
                    elif case['entry']=='scan_and_format':
                        def call():
                            return [dict(violation=v,key=v.allowlist_key,formatted=v.format())
                                for v in module.scan_file_content(**values)]
                    elif case['entry']=='counts':
                        def call():
                            rows=[module.Violation(path='a.py',line=1,col=0,severity=module.Severity(s),
                                message='',pattern_type='',name='') for s in values['severities']]
                            return module._counts_by_severity(rows)
                    elif case['entry']=='_load_allowlist_entries':
                        module.ALLOWLIST_FILE=values['allowlist_path']
                        call=lambda:module._load_allowlist_entries()
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='lazy_result_usage':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry']=='collector':
                        from lazy_results_port import collector_result
                        call=lambda:collector_result(module,values,backend if native else None)
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='transaction_patterns':
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    if case['entry'] in {'finder','internal','internal-single'}:
                        from patterns_port import probe
                        call=lambda:probe(module,case['entry'],values)
                    else:call=lambda:getattr(module,case['entry'])(**values)
                elif case['check']=='transaction_callback_wrappers' and case['entry']=='collector':
                    from callback_wrappers_port import collector_result
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    call=lambda:collector_result(module,values['path'],values['source'],values['tree'],backend if native else None,values.get('check_inline'))
                elif case['check']=='transaction_closures' and case['entry']=='probe':
                    from transaction_probe import probe
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    call=lambda:probe(module,values,backend if native else None)
                elif case['check']=='plan_scope_ownership' and case['entry']=='collector':
                    from plan_ownership_port import collector_result
                    if native:backend.item=SimpleNamespace(nodeid=case['id'])
                    call=lambda:collector_result(module,values['root'],values['path'],values['source'],values['tree'],backend if native else None)
                elif case['check'] == 'unresolved_forward_refs' and case['entry'] == 'AnnotationUsageFinder.analyze':
                    from unresolved_port import analyzed_state
                    owner = module.AnnotationUsageFinder(file_path=values['file_path'],
                        source_lines=values.get('source_lines', []), findings=values.get('initial_findings', []))
                    if native:
                        backend.item = SimpleNamespace(nodeid=case['id'])
                        action = lambda: backend.scan(case['check'], case['entry'], module, dict(self=owner, tree=values['tree']))
                    else:
                        action = lambda: owner.analyze(values['tree'])
                    def call():
                        return analyzed_state(owner, action())
                elif native and case['check'] in {'sql_grammar_escape_hatches','grammar_render_methods','grammar_str_params','grammar_callable_constructors','grammar_fragment_usage','grammar_allowlist_burndown'}:
                    backend.item=SimpleNamespace(nodeid=case['id'])
                    call=lambda:getattr(module,case['entry'])(**values)
                elif native:
                    backend.item = SimpleNamespace(nodeid=case['id'])
                    call = lambda: backend.scan(case['check'], case['entry'], module, values)
                else:
                    if case['check'] == 'backend_coupling':
                        owner = module.BackendCouplingChecker()
                        method = getattr(owner, case['entry'].split('.')[-1])
                        if case['entry'].endswith('check_file') and not case['entry'].endswith('should_check_file'):
                            import ast
                            values['tree'] = ast.parse(values['source'])
                        call = lambda: method(**values)
                    else:
                        call = lambda: getattr(module, case['entry'])(**values)
                if case['check'] in {'async_sync_delegation','sql_grammar_escape_hatches','grammar_render_methods','grammar_str_params','grammar_callable_constructors','grammar_fragment_usage','grammar_allowlist_burndown'}:
                    output,errors=io.StringIO(),io.StringIO()
                    with contextlib.redirect_stdout(output),contextlib.redirect_stderr(errors):
                        actual=invoke(call,root)
                    actual.update(stdout=output.getvalue().replace(str(root),'$ROOT'),
                                  stderr=errors.getvalue().replace(str(root),'$ROOT'))
                else:
                    actual = invoke(call, root)
                if native:
                    original = recorded['results'][index]
                    if original['id'] != case['id']:
                        raise ValueError('Fresh case identity changed')
                    try:
                        if case['check'] == 'async_sync_boundary' and case['entry'] == 'scan_tree':
                            from boundary_port import comparison_value
                            compare(comparison_value(original['value']), comparison_value(actual))
                        else:
                            compare(original['value'], actual)
                    except AssertionError:
                        report['results'].append(dict(id=case['id'], value=actual, expected=original['value'], status='failed'))
                        report.update(calls=backend.calls, programInputs=backend.inputs)
                        write_json(args.out, report)
                        os.chdir(source)
                        continue
                report['results'].append(dict(id=case['id'], value=actual, status='passed'))
                os.chdir(source)
    finally:
        os.chdir(previous_directory)
        sys.setprofile(None)
        threading.setprofile(None)
        if backend:
            backend.worker.stdin.close()
            backend.worker.wait(timeout=5)
            backend.cache.cleanup()
    if native:
        if sha(engine.read_bytes()) != report['engineSha256']:
            raise ValueError('Engine changed during fresh contracts')
        report['calls'] = backend.calls
        report['programInputs'] = backend.inputs
    for imported in tuple(sys.modules.values()):
        filename = getattr(imported, '__file__', None)
        if filename and Path(filename).suffix == '.py':
            path = Path(filename).resolve()
            if path in harness_before:
                report['harnessFiles'][path.relative_to(ROOT).as_posix()] = harness_before[path]
    if any(sha((ROOT / name).read_bytes()) != digest for name, digest in report['harnessFiles'].items()):
        raise ValueError('Harness changed during fresh contracts')
    from data_layer_probe_record import verify_sources
    report['sourceFilesChecked'] = verify_sources(source, source, PIN)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--engine', type=Path)
    parser.add_argument('--oracle', type=Path)
    parser.add_argument('--baseline', type=Path)
    parser.add_argument('--rule-root', type=Path, help='Policy directory, including disabled policies for positive controls')
    parser.add_argument('--part', type=int, default=0)
    parser.add_argument('--parts', type=int, default=1)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out = args.out.resolve()
    write_json(args.out, dict(status='started', fullMigrationAcceptance=False, results=[]))
    result = run(args)
    result['status'] = 'completed'
    write_json(args.out, result)
    failures = sum(row['status'] != 'passed' for row in result['results'])
    print(json.dumps(dict(cases=len(result['results']), failures=failures, originalPytestCases=0, execution=result['execution'])))
    if failures:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
