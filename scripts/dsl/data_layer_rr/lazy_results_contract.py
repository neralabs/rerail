"""Run all original lazy-result assertions and retain complete returned DTOs."""
import argparse
import importlib
import inspect
import os
from pathlib import Path
import subprocess
import sys
import threading

import pytest
from dto import canonical
from migration import ROOT, PIN, compare, git, read_json, sha, write_json


class Contract:
    def __init__(self, args):
        self.args = args
        self.calls, self.cases, self.collected, self.originals = [], {}, [], []
        self.fallbacks, self.depth, self.item = [], 0, None
        self.baseline = read_json(args.baseline) if args.command == 'native' else None
        self.backend = None
        if self.baseline:
            from suite import Suite
            self.backend = Suite(args.source, args.engine, ['lazy_result_usage'], args.part,
                                 args.parts, args.baseline, args.rule_root)

    def pytest_configure(self):
        from native_guard import codes
        module = importlib.import_module('scripts.lint_lazy_result_usage')
        originals = codes({'lazy_result_usage': module})
        signature = inspect.signature(module.find_violations)
        if self.backend:
            from lazy_results_port import install
            install(self.backend, module)
        original = module.find_violations
        def call(*args, **kwargs):
            root = self.item.funcargs.get('tmp_path')
            event = dict(test=self.item.nodeid, arguments=canonical(dict(signature.bind(*args, **kwargs).arguments), root))
            result = original(*args, **kwargs)
            event['return'] = canonical(result, root)
            self.calls.append(event)
            if self.baseline:
                expected = [r for r in self.baseline['calls'] if r['test'] == event['test']]
                assert len(expected) == 1
                compare(expected[0], event)
            return result
        self.originals.append((module, 'find_violations', original))
        module.find_violations = call
        if self.backend:
            def guard(frame, event, arg):
                if event == 'call' and frame.f_code in originals:
                    _, name, passive = originals[frame.f_code]
                    if not passive:
                        self.fallbacks.append(name)
                        raise AssertionError('Original semantic function invoked: ' + name)
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
        missing = [t for t in selected if sum(r['test'] == t for r in self.calls) != 1]
        result = dict(format='rerail.lazy-result-original-contracts/v1', sourcePin=PIN,
            commit=PIN, pytest_exit_code=int(code), exitCode=int(code), oracle=self.backend is None,
            source_mismatches=[], fullMigrationAcceptance=False, part=self.args.part, parts=self.args.parts,
            collected=self.collected, cases=self.cases, calls=self.calls, events=[],
            missingResults=missing, semanticFallbacks=self.fallbacks,
            sourceFiles={n: sha((self.args.source/n).read_bytes()) for n in
                ['scripts/lint_lazy_result_usage.py', 'scripts/tests/test_lint_lazy_result_usage.py']})
        if self.backend:
            result.update(engineSha256=sha(self.args.engine.read_bytes()), nativeCalls=self.backend.calls)
        write_json(self.args.out, result)
        if missing and not code:
            raise AssertionError('Original observations missing: ' + repr(missing))


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
    for name in ['scripts/lint_lazy_result_usage.py', 'scripts/tests/test_lint_lazy_result_usage.py']:
        assert (args.source/name).read_bytes() == subprocess.check_output(['git', '-C', str(args.source), 'show', PIN+':'+name])
    sys.path[:0] = [str(args.source), str(args.source/'scripts'), str(ROOT/'scripts/dsl/data_layer_audit')]
    os.chdir(args.source)
    contract = Contract(args)
    code = pytest.main(['-q', '-p', 'no:cacheprovider', '-o', 'addopts=', 'scripts/tests/test_lint_lazy_result_usage.py'], plugins=[contract])
    contract.finish(code)
    raise SystemExit(code)


if __name__ == '__main__':
    main()
