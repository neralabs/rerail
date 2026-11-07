"""Run source-derived contracts through the same native API adapters as the original assertions.

These contracts supplement the unchanged upstream suites. They never count as original pytest cases.
"""
import argparse
import importlib
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
from suite import Suite
from native_guard import codes


def arguments(values, root):
    def convert(value):
        if isinstance(value, dict) and set(value) == {'path', 'relative'} and value['relative'] is True:
            return Path(value['path'])
        if isinstance(value, dict) and set(value) == {'path'}:
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
        if isinstance(error, OSError):
            return dict(error=dict(type=type(error).__module__ + '.' + type(error).__qualname__,
                args=canonical(error.args, root), filename=canonical(error.filename, root), errno=error.errno))
        if isinstance(error, UnicodeError):
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
        casesSha256=sha(args.cases.read_bytes()), results=[], semanticFallbacks=[])
    backend = None
    harness_paths = [Path(__file__), *[Path(__file__).with_name(p) for p in ['suite.py', 'replay.py', 'dto.py', 'migration.py']],
        ROOT / 'scripts/dsl/data_layer_audit/native_guard.py']
    report['harnessFiles'] = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in harness_paths}
    harness_before = {p.resolve(): sha(p.read_bytes()) for p in (ROOT / 'scripts/dsl').rglob('*.py')}
    if native:
        engine = args.engine.resolve()
        report['engineSha256'] = sha(engine.read_bytes())
        report['oracleSha256'] = sha(args.oracle.read_bytes())
        backend = Suite(source, engine, checks, 0, 1, args.baseline.resolve())
        original_codes = codes(modules)
        def guard(frame, event, arg):
            if event == 'call' and frame.f_code in original_codes:
                check, name, passive = original_codes[frame.f_code]
                if not passive:
                    report['semanticFallbacks'].append(check + ':' + name)
                    raise AssertionError('Original detector invoked: ' + check + ':' + name)
        sys.setprofile(guard)
        threading.setprofile(guard)
    previous_directory = Path.cwd()
    os.chdir(source)
    try:
        for index, case in enumerate(corpus['cases']):
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
                if native:
                    backend.item = SimpleNamespace(nodeid=case['id'])
                    call = lambda: backend.scan(case['check'], case['entry'], module, values)
                else:
                    call = lambda: getattr(module, case['entry'])(**values)
                actual = invoke(call, root)
                if native:
                    original = recorded['results'][index]
                    if original['id'] != case['id']:
                        raise ValueError('Fresh case identity changed')
                    try:
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
