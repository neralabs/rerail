"""Record and compare complete CLI protocols independently of detector results."""
import argparse
import contextlib
import importlib
import io
import inspect
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

from migration import ROOT, PIN, compare, git, read_json, sha, write_json

CHECKS = ['backend_type_strings', 'no_source_inspection', 'hasattr_async', 'serverplan_grammar_surface']


def cases():
    result = []
    for check in CHECKS:
        for group in ['clean', 'one', 'duplicates', 'ordering']:
            for mode in ([False, True] if check in CHECKS[:2] else [False]):
                result.append(dict(check=check, id=group + ('-check' if mode else ''), group=group, args=['--check'] if mode else []))
        if check != CHECKS[-1]:
            for arg in ['--help', '--bad-flag']:
                result.append(dict(check=check, id=arg, group='clean', args=[arg]))
        if check == 'hasattr_async':
            result.append(dict(check=check, id='default-directories', group='default-directories', args=[]))
        if check == 'no_source_inspection':
            result.append(dict(check=check, id='outside-root-error', group='outside-root-error', args=[]))
            result.append(dict(check=check, id='late-outside-root-error', group='late-outside-root-error', args=[]))
    return result


def exercise(module, case, root):
    check = case['check']; calls = []
    files = [root / p for p in ['z.py', 'a/z.py', 'a-.py']]
    for p in files:
        p.parent.mkdir(parents=True, exist_ok=True); p.write_text('pass\n')
    if case['group'] == 'default-directories':
        for name in ['data_layer/new.py', 'l9999_new/new.py', 'l٠١٢٣_unicode/new.py', 'l9999_/new.py', 'impl/common/new.py']:
            p = root / name; p.parent.mkdir(parents=True, exist_ok=True); p.write_text('pass\n')
    def record(name, value):
        def call(*args, **kwargs):
            def encode(v):
                if isinstance(v, (list, tuple)): return [encode(x) for x in v]
                if isinstance(v, dict): return {k: encode(x) for k, x in v.items()}
                if isinstance(v, Path): return str(v).replace(str(root), '$ROOT')
                if hasattr(v, 'dirs'): return vars(v)
                return v
            calls.append(dict(name=name, args=encode(args), kwargs=encode(kwargs)))
            return value(*args, **kwargs) if callable(value) else value
        return call
    group = case['group']; active = group != 'clean'; duplicate = group == 'duplicates'
    with contextlib.ExitStack() as stack:
        def bind(name, value): stack.enter_context(patch.object(module, name, value))
        if check == 'backend_type_strings':
            bind('find_python_files', record('files', files))
            def scan(path):
                if not active: return []
                row = SimpleNamespace(file=path, line=12 if path == files[0] else 2, column=3,
                    code='BT002' if path == files[0] else 'BT003', message='message ☃', snippet='snippet 😀')
                return [row, row] if duplicate else [row]
            bind('check_file', record('scan', scan))
        elif check == 'no_source_inspection':
            bind('resolve_root_and_dirs', record('resolve', (root, ['tests'])))
            snippets = ['x' * 80, '😀' * 81, 'line\nwith newline']
            rows = [(p, 12-i, snippets[i], 'pattern ☃') for i, p in enumerate(files)] if active else []
            if group == 'outside-root-error': rows = [(root.parent/'outside.py', 1, 'line', 'pattern')]
            if group == 'late-outside-root-error': rows = [rows[0], (root.parent/'outside.py', 1, 'line', 'pattern')]
            bind('find_violations', record('find', rows + rows[:1] if duplicate else rows))
        elif check == 'hasattr_async':
            bind('resolve_root_and_dirs', record('resolve', (root, ['.'])))
            bind('should_exclude', record('exclude', False))
            def scan(path):
                rows = [(12, 'hasattr', 'z_async'), (2, 'getattr', '😀_async'), (2, 'hasattr', 'a_async')] if active else []
                return rows + rows[:1] if duplicate else rows
            bind('check_file', record('scan', scan))
            if group != 'default-directories': case = dict(case, args=[*case['args'], '--dirs', '.'])
        else:
            bind('ROOT', root)
            bind('iter_python_files', record('files', files))
            bind('build_serverplan_fact_engine_view', record('view', 'VIEW'))
            bind('project_serverplan_facts_from_view', record('facts', 'FACTS'))
            rows = ['z.py:12: message ☃', 'a.py:2: message 😀'] if active else []
            bind('grammar_surface_messages', record('messages', rows + rows[:1] if duplicate else rows))
        stack.enter_context(patch.object(sys, 'argv', ['lint_' + check + '.py', *case['args']]))
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try: value = dict(status=module.cli_main())
            except BaseException as error: value = dict(error=dict(type=type(error).__name__, args=[str(x).replace(str(root), '$ROOT') for x in error.args]))
    # Extractor specs are passive protocol DTOs with a stable repr.
    return dict(value, stdout=out.getvalue().replace(str(root), '$ROOT'), stderr=err.getvalue().replace(str(root), '$ROOT'),
                calls=repr(calls).replace(str(root), '$ROOT'))


def main():
    p = argparse.ArgumentParser(description=__doc__); p.add_argument('command', choices=['source', 'native'])
    for name in ['source', 'engine', 'baseline', 'out']:
        p.add_argument('--' + name, type=Path, required=name in ['source', 'out'])
    p.add_argument('--mode', choices=['snapshot', 'workspace'], default='snapshot'); a = p.parse_args()
    source = a.source.absolute(); compare(git(source, 'rev-parse', 'HEAD'), PIN)
    sys.path[:0] = [str(source), str(source/'scripts')]
    dependencies = [Path(__file__), *[Path(__file__).with_name(n) for n in ['text_cli_port.py', 'grammar_surface_port.py', 'replay.py', 'migration.py']]]
    hashes = {str(p.relative_to(ROOT)): sha(p.read_bytes()) for p in dependencies}
    source_hashes = {}
    for check in CHECKS:
        path = source / ('scripts/lint_' + check + '.py')
        compare(path.read_bytes(), subprocess.check_output(['git', '-C', str(source), 'show', PIN + ':' + str(path.relative_to(source))]))
        source_hashes[check] = sha(path.read_bytes())
    modules = {check: importlib.import_module('scripts.lint_' + check) for check in CHECKS}
    source_codes = {value.__code__ for module in modules.values() for value in vars(module).values()
                    if inspect.isfunction(value) and value.__module__ == module.__name__}
    if a.command == 'native':
        from text_cli_port import install
        from grammar_surface_port import install as surface_install
        from replay import execute_native, execute_workspace
        execute = execute_native if a.mode == 'snapshot' else execute_workspace
        class Backend:
            def __init__(self): self.calls = []
            def replace(self, owner, name, value): setattr(owner, name, value)
            def native(self, check, entry, files, options=None):
                result = execute(a.engine.absolute(), ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/(check.replace('_', '-') + '.rr'), files, dict(entryPoint=entry, **(options or {})))
                self.calls.append(result); return result['diagnostics']
        backend = Backend()
        for check, module in modules.items():
            if check == CHECKS[-1]: surface_install(backend, module)
            else: install(backend, module, check)
        baseline = read_json(a.baseline); assert baseline['passed']; compare(source_hashes, baseline['sourceHashes'])
        compare(cases(), [row['input'] for row in baseline['cases']])
        # Imported source modules provide argparse helpers and passive DTOs only.
        def guard(frame, event, _arg):
            if event == 'call' and frame.f_code in source_codes:
                raise AssertionError('Original linter semantics executed: ' + frame.f_code.co_name)
        sys.setprofile(guard)
    report = dict(format='rerail.text-cli-contract/v1', sourcePin=PIN, mode=a.command, executionMode=a.mode,
        passed=False, fullMigrationAcceptance=False, sourceHashes=source_hashes, harnessFiles=hashes, cases=[])
    if a.command == 'native': report['engineSha256'] = sha(a.engine.read_bytes())
    for i, case in enumerate(cases()):
        if a.command == 'native': sys.setprofile(guard)
        with tempfile.TemporaryDirectory(prefix='text-cli-') as temporary:
            actual = exercise(modules[case['check']], case, Path(temporary))
        row = dict(input=case)
        if a.command == 'source': row['expected'] = actual
        else:
            row.update(expected=baseline['cases'][i]['expected'], actual=actual, passed=actual == baseline['cases'][i]['expected'], native=list(backend.calls)); backend.calls.clear()
        report['cases'].append(row); write_json(a.out, report)
    if a.command == 'native': sys.setprofile(None)
    for path, digest in hashes.items(): compare(digest, sha((ROOT/path).read_bytes()))
    report['passed'] = all(row.get('passed', True) for row in report['cases']); write_json(a.out, report)
    print(dict(cases=len(report['cases']), passed=report['passed'])); raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__': main()
