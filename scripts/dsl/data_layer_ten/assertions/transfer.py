# SPDX-License-Identifier: Apache-2.0
"""Record original host assertions and their portable contracts without native scoring."""
import argparse
import ast
from contextlib import redirect_stdout, redirect_stderr
from dataclasses import asdict
import hashlib
import importlib
import io
import json
from pathlib import Path
import sys


def classify(name, expression):
    text = ast.unparse(expression)
    if name.startswith('test_main_'):
        return 'command', 'Native command arguments, output, or exit behavior'
    if 'banned_' in name:
        if 'frozenset' in text:
            return None, 'Python container type; native SQL represents membership without Python containers'
        return 'constants', 'Exact denied vocabulary and its value constraints in the shipped program'
    if name.startswith('test_violation_'):
        if 'FrozenInstanceError' in text or 'hash(' in text or 'hashable' in name or 'in_set_with_duplicates' in name:
            return None, 'Python object mutation exceptions, hashing, or set behavior'
        return 'values', 'Diagnostic field names, types, equality, or field-specific inequality'
    if any(word in text for word in ['VERSIONED_', 'GRAPH_LAZY_SQL', 'INTENT_GRAPH', 'CONDITIONAL_']) or any(word in name for word in ['_paths_are_inside_root', '_path_constants_', 'test_path_constants_']):
        return 'scope', 'Relative target identity, Python suffix, and admission in the native rule scope'
    return None, 'The pinned Python checkout or its module documentation'


class OriginalCommands:
    def __init__(self):
        self.records = {}
        self.outcomes = {}

    def pytest_runtest_setup(self, item):
        if not item.name.startswith('test_main_'):
            return
        module = item.module
        original = module.main

        def run():
            check = module.linter.run_checks
            findings = []

            def collect(*args, **kwargs):
                result = check(*args, **kwargs)
                findings.extend(asdict(value) for value in result)
                return result

            module.linter.run_checks = collect
            out, err = io.StringIO(), io.StringIO()
            exit_error = None
            result = None
            try:
                with redirect_stdout(out), redirect_stderr(err):
                    try:
                        result = original()
                    except SystemExit as error:
                        exit_error = error
            finally:
                module.linter.run_checks = check
            self.records[item.nodeid] = dict(arguments=sys.argv[1:], findings=findings, stdout=out.getvalue(), stderr=err.getvalue(),
                                              exitCode=exit_error.code if exit_error else result, raised=exit_error is not None)
            print(out.getvalue(), end='')
            print(err.getvalue(), end='', file=sys.stderr)
            if exit_error:
                raise exit_error
            return result

        module.main = run
        self.restore = module, original

    def pytest_runtest_teardown(self, item):
        if hasattr(self, 'restore'):
            module, original = self.restore
            module.main = original
            del self.restore

    def pytest_runtest_logreport(self, report):
        self.outcomes.setdefault(report.nodeid, []).append(dict(phase=report.when, outcome=report.outcome))


def clean_sources(module, tests):
    sources = {}
    for constant, factory in [('VERSIONED_LAZY_MIXIN', '_good_lazy_mixin_source'), ('VERSIONED_TRANSACTIONS', '_good_versioned_transactions_source'),
                              ('GRAPH_LAZY_SQL', '_good_graph_lazy_sql_source'), ('INTENT_GRAPH', '_good_intent_graph_source')]:
        tree = ast.parse(getattr(tests, factory)())
        for node in tree.body:
            if not isinstance(node, ast.ClassDef):
                continue
            node.body = [item for item in node.body if not isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) or item.name not in
                         {'graph_query_builder_lazy', 'graph_traverse_lazy', 'graph_query_builder', 'graph_query_builder_async', 'graph_traverse', 'graph_traverse_async'}]
            if node.name == 'SqlPlanGraphQueryBuilder':
                node.body.extend(ast.parse('@pure_lazy\ndef execute_paths_lazy(self):\n    return None\n').body)
            if node.name == 'ExecuteGraphBindingsStep':
                node.body.extend(ast.parse('def cte_column_type_hints(self):\n    return (SqlColumnTypeHints, _binding_node_sys_id_alias, _binding_edge_alias, uuid_columns, text_columns, integer_columns)\n').body)
        if constant == 'INTENT_GRAPH':
            tree.body.extend(ast.parse('class ExecuteGraphPathsStep:\n    pass\n').body)
        sources[getattr(module, constant).relative_to(module.ROOT).as_posix()] = ast.unparse(tree) + '\n'
    assert not module._violations_for_sources(sources), 'The transfer controls require a genuinely clean four-file baseline'
    return sources


def record(source, output):
    import pytest
    workspace = Path.cwd()
    sys.path[:0] = [str(source), str(source / 'scripts')]
    from data_layer_ten.common import PIN
    ledger = json.loads((workspace / 'fixtures/host/data-layer-port/batch-10-assertion-ledger.json').read_text())['cases']
    hosts = {name: value for name, value in ledger.items() if value['kind'] == 'host'}
    assert len(hosts) == 57
    collector = OriginalCommands()
    args = ['--rootdir', str(source), '-c', str(source / 'pyproject.toml'), '-q', *[str(source / name) for name in hosts]]
    from data_layer_ten.source_lock import recording_lock
    with recording_lock(source):
        exit_code = pytest.main(args, plugins=[collector])
    assert int(exit_code) == 0 and set(collector.outcomes) == set(hosts)
    for phases in collector.outcomes.values():
        assert len(phases) == 3 and all(phase['outcome'] == 'passed' for phase in phases)
    cases = {}
    for name in hosts:
        path, function = name.split('::')
        text = (source / path).read_text()
        node = next(n for n in ast.parse(text).body if isinstance(n, ast.FunctionDef) and n.name == function)
        original = ast.get_source_segment(text, node)
        assertions = []
        for item in ast.walk(node):
            expressions = [item.test] if isinstance(item, ast.Assert) else [c.context_expr for c in item.items if 'raises(' in ast.unparse(c.context_expr)] if isinstance(item, ast.With) else []
            for expression in expressions:
                contract, reason = classify(function, expression)
                assertions.append(dict(source=ast.unparse(expression), contract=contract, reason=reason))
        constructors = []
        for item in ast.walk(node):
            if isinstance(item, ast.Call) and isinstance(item.func, ast.Name) and item.func.id == 'Violation':
                values = dict(zip(['code', 'file_path', 'line_number', 'message'], map(ast.literal_eval, item.args)))
                values.update({kw.arg: ast.literal_eval(kw.value) for kw in item.keywords})
                constructors.append(values)
        cases[name] = dict(source=original, sha256=hashlib.sha256(original.encode()).hexdigest(), assertions=assertions, values=constructors,
                           **({'command': collector.records[name]} if name in collector.records else {}))
    graph = importlib.import_module('scripts.lint_graph_lazy_sql_plan')
    tests = importlib.import_module('scripts.tests.test_lint_graph_lazy_sql_plan')
    baseline = clean_sources(graph, tests)
    groups = []
    records = []
    for constant, path_constant, wrong_path in [('BANNED_VERSIONED_LAZY_SYMBOLS', 'VERSIONED_LAZY_MIXIN', 'INTENT_GRAPH'),
                                                 ('BANNED_INTENT_GRAPH_SYMBOLS', 'INTENT_GRAPH', 'VERSIONED_LAZY_MIXIN')]:
        symbols = sorted(getattr(graph, constant))
        path = getattr(graph, path_constant).relative_to(graph.ROOT).as_posix()
        groups.append(dict(name=constant, path=path, symbols=symbols))
        for symbol in symbols:
            for variant in ['positive', 'lookalike', 'other-family']:
                files = dict(baseline)
                target = getattr(graph, wrong_path).relative_to(graph.ROOT).as_posix() if variant == 'other-family' else path
                name = symbol + '_allowed' if variant == 'lookalike' else symbol
                files[target] += '\nclass ' + name + ':\n    pass\n'
                expected = [dict(code=value.code, path=value.file_path, line=value.line_number, message=value.message) for value in graph._violations_for_sources(files)]
                assert len(expected) == (1 if variant == 'positive' else 0), (symbol, variant, expected)
                records.append(dict(test='transfer-ban-' + symbol + '-' + variant, check='graph_lazy_sql_plan', entry='all', parameters={}, production=True,
                                    symbol=symbol, variant=variant, files=[dict(path=p, source=t) for p, t in files.items()], expected=expected))
    conditional = importlib.import_module('scripts.lint_conditional_architecture')
    scopes = [dict(rule='data-layer-graph-lazy-sql-plan', paths=list(baseline)),
              dict(rule='data-layer-conditional-architecture', paths=[getattr(conditional, name).relative_to(conditional.ROOT).as_posix() for name in ['CONDITIONAL_STEP_FILE', 'CONDITIONAL_DIALECT_TEST_FILE']])]
    from data_layer_probe_record import verify_sources
    checked = verify_sources(source, source, PIN)
    output.write_text(json.dumps(dict(format='rerail.data-layer-transferable-assertions/v1', sourcePin=PIN, sourceFilesChecked=checked, cases=cases, originalCaseOutcomes=collector.outcomes,
                                     bannedGroups=groups, detectorCases=records, scopes=scopes), indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    record(args.source.resolve(), args.output)
