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
import textwrap
from functools import wraps
from collections import Counter
from pathlib import Path

import pytest
from migration import ROOT, PIN, sha, git, write_json, compare, compare_inputs, fixtures_path
from dto import canonical

sys.path.insert(0, str(ROOT / 'scripts/dsl/data_layer_audit'))
from native_guard import codes

ENTRIES = {
    'removed_compat_surfaces': ['scan_file'],
    'versioned_lifecycle_legacy_surfaces': ['scan_file', 'scan_file_content'],
    'no_source_inspection': ['find_violations', 'scan_file_content'],
    'serverplan_name_inference': ['violations', '_scan_source'],
    'permission_error': ['check_file', 'check_file_content'],
    'hasattr_async': ['check_file', 'check_source', 'is_allowed_line', 'should_exclude'],
    'no_blanket_grammar_bypass': ['check_no_bypass_references', 'check_no_public_grammar_methods', 'scan_file_content'],
    'admin_async_purity': ['_collect_violations', '_collect_violations_from_source', '_should_skip', '_is_allowlisted'],
}


class Suite:
    def __init__(self, source, engine, checks, part, parts, baseline):
        self.source, self.engine, self.checks = source, engine, checks
        self.part, self.parts = part, parts
        self.originals, self.calls, self.cases, self.translations = [], [], {}, []
        self.semantic_fallbacks = []
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
        self.worker = subprocess.Popen([sys.executable, str(Path(__file__).with_name('replay.py')), '--worker', str(engine)],
            text=True, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=ROOT,
            env={**os.environ, 'RERAIL_TAPE_CACHE': str(Path(self.cache.name) / 'store.sqlite')})

    def replace(self, module, name, value):
        self.originals.append((module, name, getattr(module, name)))
        setattr(module, name, value)

    def native(self, check, entry, files, options=None):
        rule = ROOT / 'packs/data-layer/candidates/rr' / (check.replace('_', '-') + '.rr')
        request = dict(rule=str(rule), files=files, options=dict(entryPoint=entry, **(options or {})))
        call = dict(test=self.item.nodeid, check=check, entry=entry, files=len(files), status='started',
                    inputFiles=[dict(path=f['path'], sourceSha256=sha(f['source'].encode())) for f in files],
                    options=request['options'])
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
        result = response['result']
        previous = self.inputs.setdefault(check, result['programInputs'])
        if previous != result['programInputs']:
            raise AssertionError('Program changed during original assertions')
        rows = result['diagnostics']
        call.update(status='passed', findings=len(rows), diagnostics=rows,
                    programInputs=result['programInputs'], phases=result['phases'])
        return rows

    def compare_original(self, check, entry, module, args, kwargs, result):
        key = (self.item.nodeid, check, entry)
        records = [r for r in self.baseline['events'] if (r['test'], r['check'], r['entry']) == key]
        if not records:
            return
        index = self.observed[key]
        self.observed[key] += 1
        if index >= len(records):
            raise AssertionError('The original detector call count changed')
        original = records[index]
        observation = dict(test=key[0], check=check, entry=entry, ordinal=index,
                           originalRetainedReason=original.get('retained_reason'))
        self.observations.append(observation)
        from data_layer_record import ADAPTERS, snapshot, normalize
        if check == 'admin_async_purity':
            from data_layer_batch_two import normalize as normalize_two
            root = Path(args[0] if args else kwargs['project_root'])
            files = self.admin_files(module, root)
            scope = dict(include=['**/*.py'], exclude=[])
        else:
            kind = next(kind for c, e, kind in ADAPTERS if (c, e) == (check, entry))
            files, scope, root = snapshot(module, kind, args, kwargs)
        if 'sourceResult' not in original:
            raise AssertionError('A lossless original recording is required; use data_layer_rr/source.py')
        actual_source_result = canonical(result, root)
        observation.update(expectedSourceResult=original['sourceResult'], actualSourceResult=actual_source_result)
        compare(original['sourceResult'], actual_source_result)
        if original.get('retained_reason'):
            observation['status'] = 'source-result-passed; legacy normalized DTO unavailable'
            return
        compare_inputs(original, files, scope)
        actual = (normalize_two(check, result, root, files) if check == 'admin_async_purity'
                  else normalize(check, entry, result, root, files))
        observation.update(expected=original['findings'], actual=actual, status='passed')
        compare(original['findings'], actual)

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
        path = Path(arguments.get('path', arguments.get('file_path', arguments.get('filepath', 'input.py'))))
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
            root = Path(arguments.get('project_root', arguments.get('root')))
            files = []
            if entry == 'scan_file_content':
                files = [dict(path=path.relative_to(root).as_posix(), source=arguments['source'])]
            elif entry == 'check_no_public_grammar_methods':
                target = root / module.SQL_CONNECTION_FILE
                if target.exists():
                    files = [dict(path=module.SQL_CONNECTION_FILE, source=target.read_text(encoding='utf-8'))]
            else:
                for directory, names, filenames in os.walk(root):
                    names[:] = [name for name in names if name not in {'__pycache__', '.venv', '.git', 'tmp'}]
                    for name in filenames:
                        if name.endswith('.py'):
                            item = Path(directory) / name
                            try:
                                source = item.read_text(encoding='utf-8')
                            except UnicodeDecodeError:
                                continue
                            files.append(dict(path=item.relative_to(root).as_posix(), source=source))
            rows = self.native(check, entry, files, dict(sqlConnectionFile=module.SQL_CONNECTION_FILE))
            order = {f['path']: index for index, f in enumerate(files)}
            rows.sort(key=lambda r: (order.get(r['path'], -1), r['line'], r['parameters'].get('ordinal', -1), r['at']))
            if entry == 'scan_file_content':
                return [dict(file_path=r['path'], line=r['line'], message=r['message']) for r in rows]
            return [f"  {r['path']}:{r['line']}: {r['message']}" if r['line'] else f"  {r['path']}: {r['message']}" for r in rows]
        if check == 'hasattr_async':
            if entry in {'is_allowed_line', 'should_exclude'}:
                options = dict(line=arguments['line']) if entry == 'is_allowed_line' else dict(path=str(arguments['path']))
                rows = self.native(check, entry, [dict(path='input.py', source='')], options)
                code = 'ASYNC_ALLOWED' if entry == 'is_allowed_line' else 'ASYNC_EXCLUDED'
                if len(rows) > 1 or any(r['code'] != code for r in rows):
                    raise AssertionError('Invalid native helper Boolean')
                return bool(rows)
            if 'source' in arguments:
                source = arguments['source']
            else:
                try:
                    source = path.read_text()
                except Exception:
                    return []
            rows = self.native(check, entry, [dict(path=path.name, source=source)])
            rows.sort(key=lambda r: (r['line'], 0 if r['code'] == 'hasattr' else 1))
            return [(r['line'], r['code'], r['parameters']['method']) for r in rows]
        if check == 'permission_error':
            if 'source' in arguments:
                source = arguments['source']
            else:
                try:
                    source = path.read_text(encoding='utf-8')
                except (OSError, UnicodeDecodeError):
                    return []
            rows = self.native(check, entry, [dict(path=path.name, source=source)])
            rows.sort(key=lambda r: (r['line'], r['at']))
            return [module.Violation(path, r['line'], r['message']) for r in rows]
        if check in {'removed_compat_surfaces', 'versioned_lifecycle_legacy_surfaces'}:
            if 'source' in arguments:
                source = arguments['source']
            elif check == 'removed_compat_surfaces':
                try:
                    source = path.read_text(encoding='utf-8')
                except OSError:
                    return ()
            else:
                source = path.read_text()
            rows = self.native(check, entry, [dict(path=path.name, source=source)])
            rows.sort(key=lambda r: (r['line'], r['message']))
            if check == 'removed_compat_surfaces':
                location = path.relative_to(module._REPO_ROOT).as_posix() if path.is_relative_to(module._REPO_ROOT) else path.as_posix()
                return tuple(module.Violation(location, r['line'], r['code'], r['message']) for r in rows)
            return [module.Violation(path, r['parameters']['cls'], r['parameters']['method'], r['line']) for r in rows]

        if check == 'no_source_inspection':
            if entry == 'scan_file_content':
                files = [dict(path=path.as_posix().lstrip('/'),
                              source=arguments['source'])]
                originals = {files[0]['path']: path}
            else:
                root = Path(arguments['root'])
                directories = arguments.get('directories')
                directories = module.TEST_DIRECTORIES if directories is None else directories
                files, originals = [], {}
                # Path discovery and I/O are host operations. No source predicate executes here.
                for directory in directories:
                    for item in (root / directory).rglob('*.py'):
                        try:
                            source = item.read_text(encoding='utf-8')
                        except (OSError, UnicodeDecodeError):
                            continue
                        name = item.relative_to(root).as_posix()
                        files.append(dict(path=name, source=source))
                        originals[name] = item
            rows = self.native(check, entry, files)
            order = {f['path']: index for index, f in enumerate(files)}
            rows.sort(key=lambda r: (order[r['path']], r['line'], r['parameters']['ordinal']))
            if entry == 'scan_file_content':
                return [dict(file_path=str(originals[r['path']]), line=r['line'],
                             line_content=r['parameters']['snippet'], pattern_name=r['parameters']['label']) for r in rows]
            return [(originals[r['path']], r['line'], r['parameters']['snippet'], r['parameters']['label']) for r in rows]

        if check == 'serverplan_name_inference':
            if entry == '_scan_source':
                rows = self.native(check, entry, [dict(path=path.name, source=arguments['source'])])
                return [(r['line'], r['code'], r['message']) for r in sorted(rows, key=lambda r: (r['line'], r['code']))]
            paths = arguments.get('paths')
            if paths is None:
                paths = (p for root in module._SCAN_ROOTS for p in root.rglob('*.py')
                         if 'tests' not in p.parts and '__pycache__' not in p.parts)
            # The source API's public result deliberately contains basenames only.
            files, names = [], {}
            for index, item in enumerate(paths):
                name = f'input-{index}/{item.name}'
                files.append(dict(path=name, source=item.read_text()))
                names[name] = item.name
            rows = self.native(check, entry, files)
            return sorted(f"{names[r['path']]}:{r['line']}: {r['code']} {r['message']}" for r in rows)
        raise AssertionError(f'No native assertion adapter: {check}:{entry}')

    def pytest_configure(self):
        self.modules = {check: importlib.import_module('scripts.lint_' + check) for check in self.checks}
        originals = codes(self.modules)
        for check, module in self.modules.items():
            for entry in ENTRIES[check]:
                original = getattr(module, entry)
                signature = inspect.signature(original)

                def wrapper(check, entry, module, original, signature):
                    @wraps(original)
                    def run(*args, **kwargs):
                        values = dict(signature.bind(*args, **kwargs).arguments)
                        result = self.scan(check, entry, module, values)
                        self.compare_original(check, entry, module, args, kwargs, result)
                        return result
                    return run
                self.replace(module, entry, wrapper(check, entry, module, original, signature))

        def guard(frame, event, arg):
            if event != 'call' or frame.f_code not in originals:
                return
            check, name, passive = originals[frame.f_code]
            host = {'no_source_inspection': {'cli_main'}, 'hasattr_async': {'cli_main', '_default_directories'},
                    'no_blanket_grammar_bypass': {'cli_main'}, 'admin_async_purity': {'cli_main'}}
            if not passive and name not in host.get(check, set()):
                self.semantic_fallbacks.append(f'{check}:{name}')
                raise AssertionError('Original semantic execution: ' + self.semantic_fallbacks[-1])
        sys.setprofile(guard)

    def pattern_search(self, pattern, source):
        if self.item.path.name == 'test_lint_hasattr_async.py':
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
            class Match:
                def group(self, index):
                    if index != 1:
                        raise AssertionError('No native DTO contract for this capture group')
                    return method
            return Match()
        return object()

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
            if item.path.name not in {'test_lint_no_source_inspection.py', 'test_lint_hasattr_async.py', 'test_lint_admin_async_purity.py'}:
                continue
            source_lines, source_start = inspect.getsourcelines(item.obj)
            original = textwrap.dedent(''.join(source_lines))
            tree = ast.parse(original)
            function = next(n for n in tree.body if isinstance(n, ast.FunctionDef))
            function.decorator_list = []
            changed = []

            class Searches(ast.NodeTransformer):
                def visit_Call(self, node):
                    self.generic_visit(node)
                    if isinstance(node.func, ast.Attribute) and node.func.attr == 'search':
                        if len(node.args) != 1 or node.keywords:
                            raise AssertionError('Unsupported original regex assertion')
                        changed.append(node.lineno)
                        return ast.copy_location(ast.Call(func=ast.Name(id='_rr_pattern_search', ctx=ast.Load()),
                            args=[node.func.value, *node.args], keywords=[]), node)
                    return node

                def visit_Expr(self, node):
                    if (isinstance(node.value, ast.Call) and isinstance(node.value.func, ast.Attribute)
                            and isinstance(node.value.func.value, ast.Name) and node.value.func.value.id == 'pytest'
                            and node.value.func.attr == 'skip' and item.path.name == 'test_lint_hasattr_async.py'):
                        changed.append(node.lineno)
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
                         '_rr_allowed_pattern_witness': self.allowed_pattern_witness}
            exec(compile(tree, str(item.path), 'exec'), namespace)
            item.obj = namespace[function.name]
            self.translations.append(dict(test=item.nodeid, sourceSha256=sha(original.encode()),
                                          translatedSha256=sha(ast.unparse(tree).encode()),
                                          sites=[source_start + line - 1 for line in changed],
                                          operation='regex search via the native .rr pattern contract; assertions unchanged'))

    @pytest.hookimpl(tryfirst=True)
    def pytest_runtest_setup(self, item):
        self.item = item

    def pytest_runtest_logreport(self, report):
        self.cases.setdefault(report.nodeid, []).append(dict(phase=report.when, outcome=report.outcome))
        if report.when == 'teardown':
            self.item = None

    def pytest_unconfigure(self):
        sys.setprofile(None)
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
    parser.add_argument('--part', type=int, default=0)
    parser.add_argument('--parts', type=int, default=1)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--baseline', type=Path,
                        default=fixtures_path('host', 'data-layer-port', 'batch-01-recording.json'))
    args = parser.parse_args()
    source, engine, out = args.source.resolve(), args.engine.resolve(), args.out.resolve()
    harness_paths = [*Path(__file__).parent.glob('*.py'), ROOT / 'scripts/dsl/data_layer_record.py',
                     ROOT / 'scripts/dsl/data_layer_audit/native_guard.py', ROOT / 'scripts/dsl/data_layer_probe_record.py']
    harness_hashes = {str(p.relative_to(ROOT)): sha(p.read_bytes()) for p in harness_paths}
    if git(source, 'rev-parse', 'HEAD') != PIN or not 0 <= args.part < args.parts:
        parser.error('Wrong source pin or shard')
    checks = args.checks.split(',')
    if any(check not in ENTRIES for check in checks):
        parser.error('A full assertion adapter is required for each selected check')
    tests = [source / 'scripts/tests' / f'test_lint_{check}.py' for check in checks]
    source_hashes = {str(p.relative_to(source)): sha(p.read_bytes()) for p in [*tests, *[source / f'scripts/lint_{c}.py' for c in checks]]}
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
    backend = Suite(source, engine, checks, args.part, args.parts, args.baseline.resolve())
    os.chdir(source)
    code = pytest.main(['--rootdir', str(source), '-c', str(source / 'pyproject.toml'), '-q', *map(str, tests)], plugins=[backend])
    from data_layer_probe_record import verify_sources
    source_checked = verify_sources(source, source, PIN)
    wanted = Counter((r['test'], r['check'], r['entry']) for r in backend.baseline['events'] if r['test'] in backend.cases and r['check'] in checks)
    incomplete = any({p['phase']: p['outcome'] for p in phases} != dict(setup='passed', call='passed', teardown='passed')
                     for phases in backend.cases.values())
    moving_harness = any(sha((ROOT / name).read_bytes()) != digest for name, digest in harness_hashes.items())
    if sha(engine.read_bytes()) != digest or backend.semantic_fallbacks or incomplete or backend.observed != wanted or moving_harness:
        code = 1
    write_json(out, dict(format='rerail.rr-original-assertions/v1', sourcePin=PIN, oracle=False,
        engineSha256=digest, exitCode=int(code), sourceFiles=source_hashes,
        api='snapshot-cli', harnessFiles=harness_hashes,
        sourceFilesChecked=source_checked, missingResults=[dict(test=k[0], check=k[1], entry=k[2], count=n-backend.observed[k])
            for k, n in wanted.items() if n != backend.observed[k]],
        collected=backend.collected, part=args.part, parts=args.parts, cases=backend.cases,
        calls=backend.calls, assertionTranslations=backend.translations,
        originalRecordingSha256=backend.baseline_sha, detectorObservations=backend.observations,
        pythonHashSeed=os.environ.get('PYTHONHASHSEED'),
        executionIsolation='One owned persistent cache per assertion shard; shared-cache concurrency is a separate failing engine contract',
        semanticFallbacks=backend.semantic_fallbacks,
        hostOnlyCaseIds=[case for case in backend.cases if not any(c['test'] == case for c in backend.calls)]))
    raise SystemExit(code)


if __name__ == '__main__':
    main()
