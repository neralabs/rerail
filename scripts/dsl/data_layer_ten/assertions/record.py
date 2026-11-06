# SPDX-License-Identifier: Apache-2.0
"""Keep the unchanged upstream unit observations and original-evaluated controls."""
import argparse
import ast
import importlib
import json
import sys
from pathlib import Path


def record(source, report, output):
    sys.path[:0] = [str(source), str(source / 'scripts')]
    original = json.loads(report.read_text())
    assert original['oracle'] and original['exitCode'] == 0
    from data_layer_rule_audit import fixtures_path
    ledger = json.loads(fixtures_path('host', 'data-layer-port', 'batch-10-assertion-ledger.json').read_text())['cases']
    wanted = {name for name, case in ledger.items() if case['kind'] == 'native-helper'}
    assert set(original['cases']) == wanted
    assert {row['test'] for row in original['helperRecords']} == wanted
    schema = importlib.import_module('scripts.lint_schema_introspection_data_paths')
    matching = importlib.import_module('scripts.lint_matching_architecture')
    extra = []

    def add(label, check, entry, parameters, expected, text=''):
        extra.append(dict(test='supplemental:' + label, check=check, entry=entry,
                          files=[dict(path='helper.py', source=text)], parameters=parameters, expected=expected))

    for path in ['impl/common/l2070_server_plan/steps/write.py', 'l2070_batching/conditional_step.py',
                 'impl/common/l2070_server_plan/steps/tests/test_write.py',
                 'impl/common/l2070_server_plan/schema/metadata.py', 'applications/user.py']:
        add('path-' + path, 'schema_introspection_data_paths', '_is_data_path', {'path': path}, schema._is_data_path(schema.ROOT / path))
    for index, entries in enumerate([[], ['applications/user.py:1:table_exists'], ['scripts/probe.py:2:table_exists'],
                                    ['impl/common/tests/probe.py:3:table_exists'],
                                    ['impl/probe.py:1:table_exists', 'scripts/tests/probe.py:2:table_exists'],
                                    ['applications/a\x00/tests/probe.py:1:table_exists']]):
        add('approval-' + str(index), 'schema_introspection_data_paths', '_approval_entry_errors',
            {'entries': entries}, schema._approval_entry_errors(entries))
    for index, text in enumerate(['value = 1\n', 'from data_layer.matching import MatchSpec\n',
                                 'def f():\n    return value.MatchResult\n', 'from unrelated import MatchUnknown\n']):
        add('matching-' + str(index), 'matching_architecture', '_tree_uses_matching_contracts', {'owner': 0},
            matching._tree_uses_matching_contracts(ast.parse(text)), text)
    result = dict(format='rerail.data-layer-helper-assertions/v1', sourcePin='2b1504adc4291daaf080b2e932962b134da1a69a',
                  cases=original['cases'], records=original['helperRecords'], supplemental=extra)
    output.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    record(args.source.resolve(), args.report, args.output)
