#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run original upstream behavior assertions with native rule execution."""
from __future__ import annotations
import argparse
import ast
import importlib
import inspect
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import sys
from functools import wraps
from weakref import WeakKeyDictionary

import pytest

DSL = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(DSL))
from data_layer_batch_eight import BATCH_EIGHT, MODULES, TESTS, tree_source
from data_layer_probe_record import verify_sources


class NativeSuite:
    def __init__(self, root, workspace, mode, output):
        self.root, self.workspace, self.mode, self.output = root, workspace, mode, output
        self.item = None
        self.originals, self.calls, self.outcomes = [], [], {}
        self.trees = WeakKeyDictionary()
        self.original_calls = {}
        self.engine_path = Path(subprocess.check_output(['node', '--import', 'tsx', '--input-type=module', '-e',
            'import { resolveNativeBindingPath } from "./src/engine/index.ts"; console.log(resolveNativeBindingPath());'],
            cwd=workspace, text=True, timeout=15).strip())
        self.worker = subprocess.Popen(['node', '--import', 'tsx', 'conformance/data-layer/native-worker.ts'],
            cwd=workspace, text=True, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=output.with_suffix('.native.log').open('w'), env=os.environ)

    def replace(self, module, name, replacement):
        self.originals.append((module, name, getattr(module, name)))
        setattr(module, name, replacement)

    def execute(self, check, entry, files, options=None, contract=None, parameters=None):
        self.output.with_suffix('.current-call.json').write_text(json.dumps(dict(
            test=self.item.nodeid if self.item else '<setup>',check=check,entry=entry,contract=contract,
            files=len(files),sourceBytes=sum(len(file['source'].encode()) for file in files))))
        request = dict(check=check, entry=entry, files=files, options=options or {}, mode=self.mode)
        if contract is not None: request.update(contract=contract, parameters=parameters or {})
        self.worker.stdin.write(json.dumps(request) + '\n'); self.worker.stdin.flush()
        if not select.select([self.worker.stdout], [], [], 110)[0]:
            self.worker.kill(); raise RuntimeError('Native assertion backend exceeded 110 seconds')
        line = self.worker.stdout.readline()
        if not line: raise RuntimeError(f'Native assertion backend exited {self.worker.poll()}')
        response = json.loads(line)
        if 'error' in response: raise AssertionError(response['error'])
        result = response['result']
        self.last_metrics = result.get('telemetry', result.get('profile', {}))
        self.calls.append(dict(test=self.item.nodeid if self.item else '<setup>', check=check,
                               entry=entry, contract=contract, files=len(files), findings=len(result['violations']), metrics=self.last_metrics))
        return result['violations']

    def pytest_configure(self):
        original_parse, original_walk = ast.parse, ast.walk
        @wraps(original_parse)
        def parse(source, *args, **kwargs):
            tree = original_parse(source, *args, **kwargs)
            if isinstance(source, (str, bytes)):
                text = source.decode() if isinstance(source, bytes) else source
                for node in original_walk(tree): self.trees[node] = text
            return tree
        self.replace(ast, 'parse', parse)
        self.modules = {name: importlib.import_module(module) for name, module in MODULES.items()}
        from native_guard import codes, install as guard
        original_codes=codes(self.modules)
        from native_scanners import install
        install(self)
        guard(self,original_codes)

    def pytest_collection_modifyitems(self,items):
        from native_instrumentation import translate
        translate(self,items)

    @pytest.hookimpl(tryfirst=True)
    def pytest_runtest_setup(self, item):
        self.item = item
        # Independent upstream tests are independent jobs. Calls within one test still share facts.
        self.worker.stdin.write('{"reset":true}\n'); self.worker.stdin.flush()
        if not select.select([self.worker.stdout], [], [], 15)[0]:
            self.worker.kill(); raise RuntimeError('Native job reset exceeded 15 seconds')
        if json.loads(self.worker.stdout.readline()) != {'reset': True}:
            raise RuntimeError('Native backend did not reset its execution owner')

    def pytest_runtest_logreport(self, report):
        self.outcomes.setdefault(report.nodeid, []).append(dict(phase=report.when, outcome=report.outcome))
        if report.when == 'teardown': self.item = None

    def pytest_unconfigure(self):
        import threading
        sys.setprofile(None);threading.setprofile(None)
        for module, name, original in reversed(self.originals): setattr(module, name, original)
        self.worker.stdin.close()
        try: self.worker.wait(timeout=5)
        except subprocess.TimeoutExpired: self.worker.kill(); self.worker.wait()


def receipt(backend,code,policies,checked):
    sha=lambda path:hashlib.sha256(path.read_bytes()).hexdigest()
    return dict(engineSha256=sha(backend.engine_path),contractsSha256=sha(Path(os.environ['RERAIL_NATIVE_CONTRACTS'])),
      policySha256={check:sha(policies/('data-layer-'+check.replace('_','-')+'.json')) for check in BATCH_EIGHT},
      factPolicySha256={path.name:sha(path) for path in sorted((policies.parent/'facts').glob('*.json'))},
      format='rerail.data-layer-native-pytest/v1',mode=backend.mode,exitCode=int(code),instrumentationTranslations=backend.instrumentation_translations,
      sourceFilesChecked=checked,semanticFallbacks=backend.semantic_fallbacks,originalCalls=backend.original_calls,cases=backend.outcomes,calls=backend.calls)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--policies',type=Path,default=Path('packs/data-layer/candidates/rules'))
    parser.add_argument('--mode', choices=['snapshot', 'workspace'], required=True)
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    parser.add_argument('tests', nargs='*')
    args = parser.parse_args(); workspace = Path.cwd(); root = args.source.resolve(); output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    sys.path[:0] = [str(root), str(root/'scripts'), str(Path(__file__).parent)]
    policies=args.policies.resolve()
    os.environ['RERAIL_NATIVE_POLICY_DIRECTORY']=str(policies)
    os.chdir(root)
    tests = args.tests or [path for check in BATCH_EIGHT for path in TESTS[check] if (root/path).exists()]
    if not 0 <= args.part < args.parts:parser.error('part must be within [0, parts)')
    tests=tests[args.part::args.parts]
    backend = NativeSuite(root, workspace, args.mode, output)
    code = pytest.main(['--rootdir', str(root), '-c', str(root/'pyproject.toml'), '-q',
                        *[str(root/path) for path in tests]], plugins=[backend])
    if backend.semantic_fallbacks:code=pytest.ExitCode.TESTS_FAILED
    checked = verify_sources(root, args.repo.resolve(), '2b1504adc4291daaf080b2e932962b134da1a69a')
    report=receipt(backend,code,policies,checked)
    output.write_text(json.dumps(report, indent=2) + '\n')
    raise SystemExit(code)


if __name__ == '__main__': main()
