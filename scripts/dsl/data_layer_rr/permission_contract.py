"""Compare PermissionError file, discovery, and CLI contracts with the pinned source."""
import argparse
import contextlib
import importlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT, PIN, compare, git, read_json, sha, write_json

MESSAGE = 'Raises built-in PermissionError instead of l1100_exceptions.PermissionError'


def cases():
    rows = []
    for group in ['clean', 'one', 'duplicates', 'ordering', 'outside', 'late-outside']:
        for args in [[], ['--check'], ['--unrecognized', 'value']]:
            rows.append(dict(kind='cli', id=group + repr(args), group=group, args=args))
    rows.append(dict(kind='cli', id='help', group='clean', args=['--help']))
    for disk in ['pass\n', 'raise PermissionError\n',
                 'raise PermissionError; raise PermissionError()\n',
                 'raise PermissionError\nfrom l1100_exceptions import PermissionError\n',
                 'from l1100_exceptions import PermissionError\nraise PermissionError\n',
                 'from l1100_exceptions import PermissionError as Other\nraise PermissionError\n',
                 '# lint: allow-builtin-permission-error\nraise PermissionError\n',
                 'raise PermissionError(\n', None]:
        for supplied in ['pass\n', 'PermissionError', 'raise PermissionError\n']:
            rows.append(dict(kind='content', id=f'content-{len(rows)}', disk=disk, supplied=supplied))
    for excluded in [[], ['docs'], ['skip.py'], ['hidden', '.git']]:
        rows.append(dict(kind='files', id='files-' + repr(excluded), excluded=excluded))
    for value in [None, [], 'wrong', {}, {'path': 'a.py', 'line': 1},
                  *[dict(path=path, line=line, message=message) for path, line, message in [
                      ('a.py', 2, 'message'), ('./a//b.py', -3, None), ('/absolute.py', True, False),
                      ('//network/a.py', False, ['☃', 1]), ('../a.py', 3.9, {'key': 'value'}),
                      ('', -3.9, ''), (None, '  +1_024\n', 'integer string'),
                      (['a', 1], '١٢', 'Unicode decimal'), ('a.py', '1.0', 'invalid integer'),
                      ('a.py', None, 'null integer'), ('a.py', [], 'array integer'),
                      ('a.py', {}, 'object integer'), ('a.py', 2**100, 'large integer')]]]:
        rows.append(dict(kind='from-fact', id=f'from-fact-{len(rows)}', raw=value))
    for root in ['root', '.', '/', '//']:
        for path in ['root/a.py', 'root/../outside.py', '/absolute.py', '//network/a.py', '.', 'root']:
            rows.append(dict(kind='to-fact', id=f'to-fact-{len(rows)}', root=root, path=path))
    for paths in [[], ['input.py'], ['input.py', 'input.py'], ['tests/a.py'], ['test/a.py'],
                  ['test_a.py'], ['a.txt'], ['docs/a.py'], ['tmp/a.py']]:
        for relative in [False, True]:
            rows.append(dict(kind='batch', id=f'batch-{len(rows)}', paths=paths, relative=relative))
    for initial in ['pass\n', 'raise PermissionError\n']:
        for disk in ['pass\n', 'raise PermissionError\n']:
            rows.append(dict(kind='relative-file', id=f'relative-file-{len(rows)}', initial=initial, disk=disk))
    rows.append(dict(kind='relative-content', id='relative-content', disk='raise PermissionError\n'))
    return rows


def exercise(module, case, root):
    calls = []
    def clean(value):
        if isinstance(value, Path): return str(value).replace(str(root), '$ROOT')
        if isinstance(value, (tuple, list)): return [clean(v) for v in value]
        if isinstance(value, dict): return {k: clean(v) for k, v in value.items()}
        if isinstance(value, set): return sorted(value)
        return value
    def callback(name, value):
        def invoke(*args):
            calls.append(dict(name=name, args=clean(args)))
            return value
        return invoke
    out, err = io.StringIO(), io.StringIO()
    with contextlib.ExitStack() as stack:
        if case['kind'] == 'cli':
            files = [root/'z.py', root/'a/z.py', root/'a-.py']
            group = case['group']
            rows = [] if group == 'clean' else [module.Violation(p, n, 'message ☃\nsecond') for p, n in zip(files, [12, 2, 2])]
            if group == 'duplicates': rows += rows[:1]
            if group == 'outside': rows = [module.Violation(root.parent/'outside.py', 1, MESSAGE)]
            if group == 'late-outside': rows += [module.Violation(root/'../outside.py', 1, MESSAGE)]
            # Lexical '..' stays below the root for Path.relative_to; a true outside
            # path sorted after all in-root paths proves partial stdout on failure.
            if group == 'late-outside': rows[-1] = module.Violation(Path(str(root)+'z')/'outside.py', 1, MESSAGE)
            stack.enter_context(patch.object(module, 'get_project_root', callback('root', root)))
            stack.enter_context(patch.object(module, 'find_python_files', callback('files', files)))
            stack.enter_context(patch.object(module, '_violations_for_files', callback('scan', rows)))
            stack.enter_context(patch.object(sys, 'argv', ['lint_permission_error.py', *case['args']]))
            invoke = module.cli_main
        elif case['kind'] == 'content':
            path = root/'input.py'
            if case['disk'] is not None: path.write_text(case['disk'], encoding='utf-8')
            stack.enter_context(patch.object(module, 'get_project_root', lambda: root))
            invoke = lambda: [dict(path=clean(v.file_path), line=v.line_number, message=v.message)
                              for v in module.check_file_content(path, case['supplied'])]
        elif case['kind'] == 'files':
            for name in ['z.py', 'a/z.py', 'a-.py', 'docs/example.py', 'skip.py', 'hidden/x.py', '.git/x.py', 'other.txt']:
                p = root/name; p.parent.mkdir(parents=True, exist_ok=True); p.write_text('pass\n')
            invoke = lambda: clean(module.find_python_files(root, set(case['excluded'])))
        elif case['kind'] == 'from-fact':
            def invoke():
                value = module._violation_from_fact(case['raw'], root)
                return None if value is None else dict(path=clean(value.file_path), line=value.line_number, message=value.message)
        elif case['kind'] == 'to-fact':
            invoke = lambda: module._violation_to_fact(module.Violation(Path(case['path']), 2, MESSAGE), Path(case['root']))
        else:
            stack.enter_context(patch.object(module, 'get_project_root', lambda: root))
            if case['kind'] == 'batch':
                for name in case['paths']:
                    p = root/name; p.parent.mkdir(parents=True, exist_ok=True); p.write_text('raise PermissionError\n')
                inputs = tuple(Path(p) if case['relative'] else root/p for p in case['paths'])
                action = lambda: module._violations_for_files(root, inputs)
            else:
                (root/'input.py').write_text(case['disk'])
                if case['kind'] == 'relative-file':
                    caller = root/'caller'; caller.mkdir(); (caller/'input.py').write_text(case['initial'])
                    stack.enter_context(contextlib.chdir(caller))
                    action = lambda: module.check_file(Path('input.py'))
                else: action = lambda: module.check_file_content(Path('input.py'), 'PermissionError')
            invoke = lambda: [dict(path=clean(v.file_path), line=v.line_number, message=v.message) for v in action()]
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try: value = dict(value=invoke())
            except BaseException as error:
                value = dict(error=dict(type=type(error).__name__, args=[str(x).replace(str(root), '$ROOT') for x in error.args]))
    return dict(value, stdout=out.getvalue().replace(str(root), '$ROOT'),
                stderr=err.getvalue().replace(str(root), '$ROOT'), calls=calls)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    for name in ['source', 'engine', 'baseline', 'out']:
        parser.add_argument('--'+name, type=Path, required=name in ['source', 'out'])
    parser.add_argument('--mode', choices=['snapshot', 'workspace'], default='snapshot')
    parser.add_argument('--part', type=int, default=0)
    parser.add_argument('--parts', type=int, default=1)
    parser.add_argument('--kind', action='append', help='Run only the specified contract kinds')
    args = parser.parse_args(); source = args.source.absolute()
    if not 0 <= args.part < args.parts: parser.error('Expected 0 <= part < parts')
    compare(git(source, 'rev-parse', 'HEAD'), PIN)
    path = source/'scripts/lint_permission_error.py'
    compare(path.read_bytes(), subprocess.check_output(['git', '-C', str(source), 'show', PIN+':scripts/lint_permission_error.py']))
    sys.path[:0] = [str(source), str(source/'scripts')]
    module = importlib.import_module('scripts.lint_permission_error')
    if args.command == 'native':
        from permission_port import install
        from replay import execute_native, execute_workspace
        from policy_routes import policy_filename
        from data_layer_audit.native_guard import codes
        original_codes = codes({'permission_error': module})
        execute = execute_native if args.mode == 'snapshot' else execute_workspace
        class Backend:
            def __init__(self): self.calls = []
            def replace(self, owner, name, value): setattr(owner, name, value)
            def native(self, check, entry, files, options=None):
                result = execute(args.engine.absolute(), ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/policy_filename(check, entry),
                                 files, dict(entryPoint=entry, **(options or {})))
                self.calls.append(result); return result['diagnostics']
        backend = Backend(); install(backend, module, detectors=True)
        baseline = read_json(args.baseline); assert baseline['passed']
        compare(sha(path.read_bytes()), baseline['sourceSha256'])
        compare(cases(), [r['input'] for r in baseline['cases']])
        def guard(frame, event, _arg):
            if event == 'call' and frame.f_code in original_codes and not original_codes[frame.f_code][2]:
                raise AssertionError('Original linter semantics executed: '+frame.f_code.co_name)
    dependencies = [Path(__file__), *[Path(__file__).with_name(n) for n in
                    ['replay.py', 'migration.py', 'policy_routes.py', 'permission_port.py'] if Path(__file__).with_name(n).exists()]]
    hashes = {str(p.relative_to(ROOT)): sha(p.read_bytes()) for p in dependencies}
    report = dict(format='rerail.permission-contract/v1', sourcePin=PIN, mode=args.command,
                  executionMode=args.mode, passed=False, fullMigrationAcceptance=False,
                  sourceSha256=sha(path.read_bytes()), harnessFiles=hashes, cases=[])
    report.update(part=args.part, parts=args.parts, kinds=args.kind, totalCases=len(cases()))
    if args.command == 'native': report.update(engineSha256=sha(args.engine.read_bytes()), oracle=False)
    for i, case in enumerate(cases()):
        if i % args.parts != args.part or (args.kind and case['kind'] not in args.kind): continue
        if args.command == 'native': sys.setprofile(guard)
        try:
            with tempfile.TemporaryDirectory(prefix='permission-contract-') as temporary:
                value = exercise(module, case, Path(temporary))
        finally: sys.setprofile(None)
        row = dict(input=case)
        if args.command == 'source': row['expected'] = value
        else:
            row.update(expected=baseline['cases'][i]['expected'], actual=value,
                       passed=value == baseline['cases'][i]['expected'], native=list(backend.calls))
            backend.calls.clear()
        report['cases'].append(row); write_json(args.out, report)
    for name, digest in hashes.items(): compare(digest, sha((ROOT/name).read_bytes()))
    report['passed'] = all(r.get('passed', True) for r in report['cases'])
    write_json(args.out, report)
    print(dict(cases=len(report['cases']), passed=report['passed']))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__': main()
