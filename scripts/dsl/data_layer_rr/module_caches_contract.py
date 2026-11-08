"""Execute every original module-cache assertion and record complete API and property values."""
import argparse
import ast
import importlib
import inspect
import os
from pathlib import Path
import subprocess
import sys
import threading

import pytest
from ast_transport import visit_columns
from dto import canonical
from migration import ROOT, PIN, compare, git, read_json, sha, write_json

FUNCTIONS = ['scan_codebase', '_scan_dirs', '_load_allowlist_entries', '_load_allowlist_index',
             'scan_file_content', '_scan_file', '_collect_files', '_facts_for_files', '_counts_by_severity']
PROPERTIES = {'Violation': ['allowlist_key'], 'AllowlistEntry': ['match_key', 'signature_entry']}


class Contract:
    def __init__(self, args):
        self.args = args
        self.calls, self.cases, self.collected, self.originals = [], {}, [], []
        self.fallbacks, self.depth, self.item, self.observed = [], 0, None, {}
        self.baseline = read_json(args.baseline) if args.command == 'native' else None
        self.backend = None
        names = ['module_caches_port.py', 'ast_transport.py', 'suite.py', 'replay.py', 'dto.py', 'migration.py']
        paths = [Path(__file__), *[Path(__file__).with_name(n) for n in names], ROOT/'scripts/dsl/data_layer_audit/native_guard.py']
        self.harness = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in paths if p.is_file()}
        if self.baseline:
            from suite import Suite
            self.backend = Suite(args.source, args.engine, ['module_level_caches'], args.part,
                                 args.parts, args.baseline, args.rule_root)

    def value(self, value):
        if isinstance(value, ast.AST):
            return dict(syntax=visit_columns(value))
        if isinstance(value, dict):
            return dict(kind='dict', items=[[self.value(k), self.value(v)] for k, v in value.items()])
        if isinstance(value, list):
            return [self.value(v) for v in value]
        return canonical(value, self.item.funcargs.get('tmp_path') if self.item else None)

    def pytest_configure(self):
        from native_guard import codes
        module = importlib.import_module('scripts.lint_module_level_caches')
        originals = codes({'module_level_caches': module})
        targets = [(module, n, None) for n in FUNCTIONS]
        targets += [(getattr(module, c), n, c) for c, names in PROPERTIES.items() for n in names]
        signatures = {(owner, n): inspect.signature(getattr(owner, n).fget if c else getattr(owner, n))
                      for owner, n, c in targets}
        if self.backend:
            from module_caches_port import install
            install(self.backend, module)
        for owner, name, cls in targets:
            original = getattr(owner, name)
            function = original.fget if cls else original
            signature = signatures[owner, name]
            entry = cls+'.'+name if cls else name
            def wrap(function, signature, entry):
                def call(*args, **kwargs):
                    if self.depth:
                        return function(*args, **kwargs)
                    event = dict(test=self.item.nodeid, entry=entry,
                        arguments=self.value(dict(signature.bind(*args, **kwargs).arguments)))
                    self.depth += 1
                    try:
                        result = function(*args, **kwargs)
                        event['return'] = self.value(result)
                        return result
                    except Exception as error:
                        event['error'] = dict(type=type(error).__module__+'.'+type(error).__qualname__, args=self.value(error.args))
                        raise
                    finally:
                        self.depth -= 1
                        self.calls.append(event)
                        if self.baseline:
                            expected = [r for r in self.baseline['calls'] if r['test']==event['test']]
                            i = self.observed.get(event['test'], 0)
                            self.observed[event['test']] = i+1
                            compare(expected[i], event)
                return call
            self.originals.append((owner, name, original))
            function = wrap(function, signature, entry)
            setattr(owner, name, property(function) if cls else function)
        if self.backend:
            def guard(frame, event, arg):
                if event == 'call' and frame.f_code in originals:
                    _, name, passive = originals[frame.f_code]
                    if not passive:
                        self.fallbacks.append(name)
                        raise AssertionError('Original semantic function invoked: '+name)
            sys.setprofile(guard)
            threading.setprofile(guard)

    def pytest_collection_modifyitems(self, items):
        self.collected = [i.nodeid for i in items]
        if self.baseline:
            compare(self.baseline['collected'], self.collected)
        items[:] = items[self.args.part::self.args.parts]

    def pytest_runtest_setup(self, item):
        self.item = item
        if self.backend:
            self.backend.item = item

    def pytest_runtest_logreport(self, report):
        self.cases.setdefault(report.nodeid, []).append(dict(phase=report.when, outcome=report.outcome))

    def pytest_unconfigure(self):
        sys.setprofile(None)
        threading.setprofile(None)
        for owner, name, value in reversed(self.originals):
            setattr(owner, name, value)

    def finish(self, code):
        if self.backend:
            self.backend.worker.stdin.close()
            self.backend.worker.wait(timeout=10)
            self.backend.cache.cleanup()
        selected = self.collected[self.args.part::self.args.parts]
        missing = [t for t in selected if self.baseline and sum(r['test']==t for r in self.baseline['calls']) != self.observed.get(t, 0)]
        for name, digest in self.harness.items():
            assert sha((ROOT/name).read_bytes()) == digest, 'Harness changed: '+name
        result = dict(format='rerail.module-caches-original-contracts/v1', sourcePin=PIN, commit=PIN,
            pytest_exit_code=int(code), exitCode=int(code), oracle=self.backend is None, source_mismatches=[],
            fullMigrationAcceptance=False, part=self.args.part, parts=self.args.parts, collected=self.collected,
            cases=self.cases, calls=self.calls, events=[], missingResults=missing,
            semanticFallbacks=self.fallbacks, harnessFiles=self.harness,
            sourceFiles={n: sha((self.args.source/n).read_bytes()) for n in
                ['scripts/lint_module_level_caches.py', 'scripts/tests/test_lint_module_level_caches.py']})
        if self.backend:
            result.update(engineSha256=sha(self.args.engine.read_bytes()), nativeCalls=self.backend.calls)
        write_json(self.args.out, result)
        if missing and not code:
            raise AssertionError('Original observations missing: '+repr(missing))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    for name in ['source', 'out', 'baseline', 'engine', 'rule-root']:
        parser.add_argument('--'+name, type=Path, required=name in ['source', 'out'])
    parser.add_argument('--part', type=int, default=0)
    parser.add_argument('--parts', type=int, default=1)
    args = parser.parse_args()
    for name in ['source', 'out', 'baseline', 'engine', 'rule_root']:
        if getattr(args, name) is not None:
            setattr(args, name, getattr(args, name).absolute())
    assert git(args.source, 'rev-parse', 'HEAD') == PIN and 0 <= args.part < args.parts
    for name in ['scripts/lint_module_level_caches.py', 'scripts/tests/test_lint_module_level_caches.py']:
        assert (args.source/name).read_bytes() == subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name])
    sys.path[:0] = [str(args.source), str(args.source/'scripts'), str(ROOT/'scripts/dsl/data_layer_audit')]
    os.chdir(args.source)
    contract = Contract(args)
    code = pytest.main(['-q','-p','no:cacheprovider','-o','addopts=','scripts/tests/test_lint_module_level_caches.py'], plugins=[contract])
    contract.finish(code)
    raise SystemExit(code)


if __name__ == '__main__':
    main()
