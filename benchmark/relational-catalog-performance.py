#!/usr/bin/env python3
"""Fail on missing coverage, changed answers, or VM regressions in the catalog."""
import argparse
import json
from pathlib import Path


def check(report, budget):
    actual = {(c['policy'], c['clause'], g['grammar']): g['result']
              for c in report['clauses'] for g in c.get('grammars', [])}
    errors = []
    budgeted = {(b['policy'], b['clause'], b['grammar']) for b in budget['budgets']}
    for key, result in actual.items():
        if result.get('admitted') and key not in budgeted:
            errors.append(f'{key}: admitted grammar has no pinned performance budget')
    if report['fixtures'] != budget['fixtures']:
        errors.append('Fixture inventory differs from the pinned budget')
    for limit in budget['budgets']:
        key = (limit['policy'], limit['clause'], limit['grammar'])
        result = actual.get(key, {})
        execution = result.get('execution', {})
        if not result.get('admitted') or not execution.get('passed'):
            errors.append(f'{key}: grammar missing or parity failed')
            continue
        for field in ['nativeOutputSha256', 'framePublications']:
            if execution[field] != limit[field]:
                errors.append(f'{key}: {field} changed')
        # The final matching count stays pinned. Explicit physical cuts add
        # schema/reset/population executions, all charged to the same VM budget.
        # Whole-job packing has its own native-parity and <=100 execution gate.
        stages = execution.get('stageExecution')
        if stages:
            records = stages.get('statements', [])
            stage_count = sum(row.get('matchingStatements', 0) for row in records)
            final_count = execution.get('finalStatement', {}).get('matchingStatements')
            valid = (records and all(row.get('matchingStatements') == 1 for row in records)
                     and stages.get('stageExecutions') == stage_count
                     and final_count == limit['matchingStatements']
                     and stages.get('expectedMatchingStatements') == stage_count + final_count
                     and execution['matchingStatements'] == stage_count + final_count
                     and execution['matchingStatements'] <= 100)
            stage_vm = [row.get('vmSteps') for row in records]
            final_vm = execution.get('finalStatement', {}).get('vmSteps')
            accounted_vm = (all(type(vm) is int and vm >= 0 for vm in stage_vm)
                            and type(final_vm) is int and final_vm >= 0
                            and stages.get('vmSteps') == sum(stage_vm)
                            and execution['vm'] == final_vm + sum(stage_vm))
            if not accounted_vm:
                errors.append(f'{key}: VM total does not account for every physical stage')
        else:
            valid = execution['matchingStatements'] == limit['matchingStatements']
        if not valid:
            errors.append(f'{key}: matchingStatements changed without fully accounted physical cuts')
        if execution['vm'] > limit['maxVm']:
            errors.append(f"{key}: VM {execution['vm']} exceeds {limit['maxVm']}")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('report', type=Path)
    parser.add_argument('--budget', type=Path, default=Path(__file__).with_name(
        'sql-relational-catalog-performance-budget.json'))
    args = parser.parse_args()
    errors = check(json.loads(args.report.read_text()), json.loads(args.budget.read_text()))
    print(json.dumps({'passed': not errors, 'failures': errors}, indent=2))
    raise SystemExit(bool(errors))


if __name__ == '__main__':
    main()
