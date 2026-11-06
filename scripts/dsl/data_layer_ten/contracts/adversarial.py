# SPDX-License-Identifier: Apache-2.0
"""Observe configured reporters after moving declarations across AST boundaries."""
import json
from pathlib import Path
import textwrap


def record():
    from data_layer_ten.lifecycle import observe
    from data_layer_nine.evidence import expand
    from data_layer_ten.evidence import read_report
    fixture = Path('fixtures/host/data-layer-port/batch-10-configured-defaults.json.gz')
    original = expand(read_report(fixture))['records']
    seeds = json.loads(Path('fixtures/host/data-layer-port/batch-10-shape-seeds.json').read_text())
    cases = []
    seen = set()
    for seed in seeds['records']:
        check = seed['check']
        files = [dict(path=f['path'], source=seeds['sources'][f['sourceSha256']]) for f in seed['files']]
        # Keep upstream scopes. Also test source-only probes in a real owner file.
        owners = {'graphql_plan_ownership': 'l9750_graphql/probe.py',
                  'generated_index_sources': 'impl/common/l7245_versioned_backends/generic_sparse.py',
                  'hnsw_architecture': 'impl/common/l2100_sql_grammar/domain/sql_table_hnsw/algorithm.py'}
        if len(files) == 1 and check in owners:
            files[0]['path'] = owners[check]
        for label, header in [('guard', 'if flag:'), ('class', 'class Envelope:'), ('function', 'def envelope():')]:
            changed = [dict(path=f['path'], source=header + '\n' + textwrap.indent(f['source'], '    '))
                       if f['path'].endswith('.py') else f for f in files]
            identity = (check, json.dumps(changed, sort_keys=True))
            if identity in seen:
                continue
            seen.add(identity)
            cases.append(dict(test=seed['test'] + '-' + label, check=check, entry='all', parameters={},
                              production=True, files=changed, expected=observe(check, changed)))
    # Each detector needs both real positive and clean configured controls.
    for check in dict.fromkeys(row['check'] for row in seeds['records']):
        for variant in ['positive', 'clean']:
            seed = next(row for row in original if row['test'] == check + ':' + variant)
            cases.append(dict(test=check + '-control-' + variant, check=check, entry='all', parameters={},
                              production=True, files=seed['files'], expected=observe(check, seed['files'])))
    return cases
