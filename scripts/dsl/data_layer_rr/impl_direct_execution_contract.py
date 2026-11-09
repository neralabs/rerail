"""Execute every original impl-direct-execution assertion and record complete API and property values."""
import argparse
import ast
import importlib
import inspect
import os
import contextlib
from dataclasses import fields, is_dataclass
import io
from pathlib import Path
import subprocess
import sys
import threading

import pytest
from ast_transport import visit_columns
from dto import canonical
from migration import ROOT, PIN, compare, git, read_json, sha, write_json

FUNCTIONS = ['is_allowed_file','scan_file','scan_source','scan_fts_manager_direct_execution','scan_fts_manager_source','scan_directories','scan_directory','cli_main']
CLASSES = []
PROPERTIES = {}



class Contract:
    def __init__(self, args):
        self.args = args
        self.calls, self.cases, self.collected, self.originals = [], {}, [], []
        self.module=None
        self.selected=[]
        self.fallbacks, self.depth, self.item, self.observed = [], 0, None, {}
        self.host_protocol_calls = []
        self.baseline = read_json(args.baseline) if args.command == 'native' else None
        self.backend = None
        names = ['impl_direct_execution_port.py', 'ast_sources.py', 'ast_transport.py', 'suite.py', 'replay.py', 'dto.py', 'migration.py']
        paths = [Path(__file__), *[Path(__file__).with_name(n) for n in names], ROOT/'scripts/dsl/data_layer_audit/native_guard.py']
        self.harness = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in paths if p.is_file()}
        if self.baseline:
            from suite import Suite
            self.backend = Suite(args.source, args.engine, ['impl_direct_execution'], args.part,
                                 args.parts, args.baseline, args.rule_root, execution_mode=args.mode)

    def value(self, value):
        root=self.item.funcargs.get('tmp_path') if self.item else None
        if isinstance(value,str) and root is not None:value=value.replace(str(root),'$ROOT')
        if type(value).__name__ == 'FactEngine':
            return dict(type='FactEngine',root=self.value(value.root))
        if type(value).__name__ == 'StructuredLinterResult':
            return dict(type='StructuredLinterResult',name=value.name,enforcement=value.enforcement,violations=self.value(value.violations),output_blocks=self.value(value.output_blocks))
        if inspect.isclass(value) and value.__module__==self.module.__name__:
            return dict(type='class',name=value.__name__)
        if type(value).__name__=='LinterPlugin':
            return dict(type='LinterPlugin',name=value.name,enforcement=value.enforcement,description=value.description,
                file_scope=self.value(value.file_scope),required_facts=self.value(value.required_facts),reporter=callable(value.reporter))
        if any(isinstance(value,getattr(self.module,n)) for n in CLASSES):return dict(type=type(value).__name__,state=self.value(vars(value)))
        if isinstance(value, ast.AST):
            return dict(syntax=visit_columns(value))
        if is_dataclass(value) and not isinstance(value,type):
            return dict(kind='dataclass',type=type(value).__module__+'.'+type(value).__qualname__,fields={f.name:self.value(getattr(value,f.name)) for f in fields(value)})
        if isinstance(value, tuple):
            return dict(kind='tuple',items=[self.value(v) for v in value])
        if isinstance(value, dict):
            return dict(kind='dict', items=[[self.value(k), self.value(v)] for k, v in value.items()])
        if isinstance(value, list):
            return [self.value(v) for v in value]
        return canonical(value, self.item.funcargs.get('tmp_path') if self.item else None)

    def pytest_configure(self):
        from native_guard import codes
        module = importlib.import_module('scripts.lint_impl_direct_execution')
        self.module=module
        originals = codes({'impl_direct_execution': module})
        targets = [(module,n,None) for n in FUNCTIONS]
        targets += [(getattr(module,c),n,None) for c in CLASSES for n,f in vars(getattr(module,c)).items() if inspect.isfunction(f) and f.__code__.co_filename == str(self.args.source / "scripts/lint_impl_direct_execution.py")]
        targets += [(getattr(module, c), n, c) for c, names in PROPERTIES.items() for n in names]
        signatures = {(owner, n): inspect.signature(getattr(owner, n).fget if c else getattr(owner, n))
                      for owner, n, c in targets}
        if self.backend:
            from impl_direct_execution_port import install
            install(self.backend, module)
        for owner, name, cls in targets:
            original = getattr(owner, name)
            function = original.fget if cls else original
            signature = signatures[owner, name]
            entry = cls+'.'+name if cls else name if owner is module else owner.__name__+'.'+name
            def wrap(function, signature, entry):
                def call(*args, **kwargs):
                    if self.depth:
                        return function(*args, **kwargs)
                    event = dict(test=self.item.nodeid, entry=entry,
                        arguments=self.value(dict(signature.bind(*args, **kwargs).arguments)))
                    self.depth += 1
                    output, errors = io.StringIO(), io.StringIO()
                    try:
                        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
                            result = function(*args, **kwargs)
                        event['return'] = self.value(result)
                        return result
                    except BaseException as error:
                        event['error'] = dict(type=type(error).__module__+'.'+type(error).__qualname__, args=self.value(error.args))
                        raise
                    finally:
                        self.depth -= 1
                        root = self.item.funcargs.get('tmp_path') if self.item else None
                        for name, stream, target in [('stdout', output, sys.stdout), ('stderr', errors, sys.stderr)]:
                            raw = stream.getvalue()
                            event[name] = raw.replace(str(root), '$ROOT') if root else raw
                            target.write(raw)
                        if args and any(isinstance(args[0],getattr(self.module,n)) for n in CLASSES):
                            event['receiverAfter'] = self.value(args[0])
                        self.calls.append(event)
                        if self.baseline:
                            expected = [r for r in self.baseline['calls'] if r['test']==event['test']]
                            i = self.observed.get(event['test'], 0)
                            self.observed[event['test']] = i+1
                            compare(expected[i], event)
                call.__signature__ = signature
                return call
            self.originals.append((owner, name, original))
            function = wrap(function, signature, entry)
            setattr(owner, name, property(function) if cls else function)
        if self.backend:
            def guard(frame, event, arg):
                if event == 'call' and frame.f_code in originals:
                    _, name, passive = originals[frame.f_code]
                    host_protocol = False
                    if host_protocol:
                        self.host_protocol_calls.append(dict(test=self.item.nodeid, entry=name))
                    if not passive and not host_protocol:
                        self.fallbacks.append(name)
                        raise AssertionError('Original semantic function invoked: '+name)
            self.guard = guard
            sys.setprofile(guard)
            threading.setprofile(guard)

    def pytest_collection_modifyitems(self, items):
        self.collected = [i.nodeid for i in items]
        if self.baseline:
            compare(self.baseline['collected'], self.collected)
        selected=items[self.args.part::self.args.parts]
        if self.args.entries:
            if not self.baseline:raise ValueError('Record the full source suite before selecting native helper slices')
            wanted=set(self.args.entries.split(','))
            selected=[item for item in selected if (calls:=[r for r in self.baseline['calls'] if r['test']==item.nodeid]) and all(r['entry'] in wanted for r in calls)]
        self.selected=[item.nodeid for item in selected]
        items[:]=selected

    def pytest_runtest_setup(self, item):
        self.item = item
        if self.backend:
            self.backend.item = item
            # CPython clears a profile callback that raises. A failed test must
            # not disable source-call detection in every subsequent test.
            sys.setprofile(self.guard)
            threading.setprofile(self.guard)

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
        selected = self.selected
        missing = [t for t in selected if self.baseline and sum(r['test']==t for r in self.baseline['calls']) != self.observed.get(t, 0)]
        for name, digest in self.harness.items():
            assert sha((ROOT/name).read_bytes()) == digest, 'Harness changed: '+name
        result = dict(format='rerail.impl-direct-execution-original-contracts/v1', sourcePin=PIN, commit=PIN,
            pytest_exit_code=int(code), exitCode=int(code), oracle=self.backend is None, source_mismatches=[],
            fullMigrationAcceptance=False, mode=self.args.mode if self.backend else 'source', part=self.args.part, parts=self.args.parts, collected=self.collected, selectedCases=self.selected, selectedEntries=self.args.entries,
            cases=self.cases, calls=self.calls, events=[], missingResults=missing,
            semanticFallbacks=self.fallbacks, hostProtocolCalls=self.host_protocol_calls, harnessFiles=self.harness,
            sourceFiles={n: sha((self.args.source/n).read_bytes()) for n in
                ['scripts/lint_impl_direct_execution.py', 'scripts/tests/test_lint_impl_direct_execution.py']})
        if self.backend:
            result.update(engineSha256=sha(self.args.engine.read_bytes()), nativeCalls=self.backend.calls,policySha256={'impl_direct_execution':sha((self.backend.rule_root/'impl-direct-execution.rr').read_bytes())})
        write_json(self.args.out, result)
        if missing and not code:
            raise AssertionError('Original observations missing: '+repr(missing))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    for name in ['source', 'out', 'baseline', 'engine', 'rule-root']:
        parser.add_argument('--'+name, type=Path, required=name in ['source', 'out'])
    parser.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot')
    parser.add_argument('--entries',help='Explicit native helper slice; does not certify the complete original suite')
    parser.add_argument('--part', type=int, default=0)
    parser.add_argument('--parts', type=int, default=1)
    args = parser.parse_args()
    for name in ['source', 'out', 'baseline', 'engine', 'rule_root']:
        if getattr(args, name) is not None:
            setattr(args, name, getattr(args, name).absolute())
    assert git(args.source, 'rev-parse', 'HEAD') == PIN and 0 <= args.part < args.parts
    for name in ['scripts/lint_impl_direct_execution.py', 'scripts/tests/test_lint_impl_direct_execution.py']:
        assert (args.source/name).read_bytes() == subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name])
    sys.path[:0] = [str(args.source), str(args.source/'scripts'), str(ROOT/'scripts/dsl/data_layer_audit')]
    os.chdir(args.source)
    contract = Contract(args)
    code = pytest.main(['-q','-p','no:cacheprovider','-o','addopts=','scripts/tests/test_lint_impl_direct_execution.py'], plugins=[contract])
    contract.finish(code)
    raise SystemExit(code)


if __name__ == '__main__':
    main()
