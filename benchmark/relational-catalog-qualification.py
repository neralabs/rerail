#!/usr/bin/env python3
"""Require complete catalog coverage, native parity, and fixed VM budgets."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    sources = sorted((root / 'native/src/code_rule_dsl').rglob('*.rs'))
    sources += sorted((root / 'packs/python/rules').glob('*.json'))
    for tool in ['ruff', 'bandit']:
        sources += sorted(p for p in (root / f'packs/python/upstream/{tool}/fixtures').rglob('*')
                          if p.suffix in ['.py', '.pyi'])
    sources += [root / 'benchmark/relational-catalog-performance.py',
                root / 'benchmark/sql-relational-catalog-performance-budget.json',
                Path(__file__).resolve()]
    pinned = {str(p.relative_to(root)): digest(p) for p in sources}
    executable = digest(binary)
    report = output / 'catalog.json'
    env = dict(os.environ, RUST_MIN_STACK='67108864',
               RERAIL_RELATIONAL_CUTOVER_LEDGER=str(report),
               RERAIL_RELATIONAL_CUTOVER_SQL_DIR=str(output / 'sql'))
    with (output / 'catalog.log').open('w') as log:
        run = subprocess.run([str(binary), 'relational_cutover_catalog_fixture_parity',
                              '--ignored', '--nocapture', '--test-threads=1'],
                             cwd=root, env=env, stdout=log, stderr=log, timeout=900)
    catalog = json.loads(report.read_text())
    summary = catalog['summary']
    coverage = (summary['admitted'] == summary['grammars'] == 146
                and summary['executionPassed'] == 146
                and summary['executionFailed'] == 0
                and summary['noPositiveWitness'] == 0
                and summary['policyCompilationErrors'] == 0)
    spec = importlib.util.spec_from_file_location('performance_gate',
        root / 'benchmark/relational-catalog-performance.py')
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    budget = json.loads((root / 'benchmark/sql-relational-catalog-performance-budget.json').read_text())
    errors = gate.check(catalog, budget)
    shape_errors = []
    resource_errors = []
    compiler_scopes = {}
    ordered_checked = 0
    set_scopes = {
        "capture-free total relations, structural components, ancestor closure, and proved subtree demand",
        "Complete ordered single-leaf answer plan",
    }
    for clause in catalog['clauses']:
        for grammar in clause.get('grammars', []):
            result = grammar['result']
            execution = result.get('execution', {})
            if (execution.get('matchingStatements') != 1
                    or execution.get('framePublications') != 0):
                shape_errors.append([clause['policy'], clause['clause'], grammar['grammar']])
            lowering = result.get('lowering') or []
            operators = result.get('compositionOperators') or []
            returns = result.get('nestedReturnCells') or []
            scope = result.get('compilerScope')
            compiler_scopes[scope or 'missing'] = compiler_scopes.get(scope or 'missing', 0) + 1
            within_budget = result['sqlBytes'] <= 256000
            if scope in {
                    'Workspace clause/file failure publication kernel',
                    'Explicit outcomes with reached node-local value leaves and traversal stops'}:
                ordered_checked += 1
                within_budget = within_budget and bool(lowering and operators and returns) and (
                    all(p['loweredOperators'] <= 256
                        and p.get('finiteControlOperators', 0) <= 256 for p in lowering)
                    and max(operators) <= 256 and max(returns) <= 128)
            elif scope not in set_scopes:
                within_budget = False
            if not within_budget:
                resource_errors.append([clause['policy'], clause['clause'], grammar['grammar']])
    unchanged = executable == digest(binary) and all(
        digest(root / path) == value for path, value in pinned.items())
    result = {'scope': 'Complete catalog grammar coverage, native fixture parity, and existing VM ceilings.',
              'notQualified': ['Native physical stack-byte equivalence',
                               'Production policy publication and incremental invalidation',
                               'Complete-job batching within 100 executions',
                               'Aggregate storage-byte limits and deadlines'],
              'passed': run.returncode == 0 and coverage and not errors and not shape_errors and not resource_errors and unchanged,
              'catalogExitCode': run.returncode, 'completeCoveragePassed': coverage,
              'performancePassed': not errors, 'performanceFailures': errors,
              'statementShapePassed': not shape_errors, 'statementShapeFailures': shape_errors,
              'resourceBudgetsPassed': not resource_errors,
              'resourceContract': {'allGrammars': 'SQL bytes <= 256000',
                                   'orderedGrammars': 'Source and finite IR and emitted operators <= 256; typed return cells <= 128',
                                   'orderedGrammarsChecked': ordered_checked,
                                   'compilerScopes': compiler_scopes}, 'resourceBudgetFailures': resource_errors,
              'inputsUnchanged': unchanged, 'summary': summary,
              'sourceHashes': pinned, 'executableSha256': executable}
    (output / 'evidence.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: v for k, v in result.items()
                      if k not in ['sourceHashes', 'summary']}, indent=2))
    raise SystemExit(not result['passed'])


if __name__ == '__main__':
    main()
