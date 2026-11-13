#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Record fifth-batch source probes against the verified upstream analyzers."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import importlib
import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

from data_layer_batch_five import BATCH_FIVE, MODULES, normalize
from data_layer_probe_record import verify_sources

PIN = '2b1504adc4291daaf080b2e932962b134da1a69a'
GRAPH_CHECKS = ('public_api_triples', 'internal_dead_code', 'imperative_private_functions')


def record(case, source_root):
    check = case['check']
    module = importlib.import_module(MODULES[check])
    with tempfile.TemporaryDirectory(prefix='rerail-dl-batch-five-') as directory, ExitStack() as context:
        root = Path(directory).resolve()
        files = case['files']
        for value in files:
            path = root/value['path']
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(value['source'])
        for name in ('ROOT', 'REPO_ROOT', 'PROJECT_ROOT'):
            if hasattr(module, name):context.enter_context(patch.object(module, name, root))
        path = root/files[0]['path']
        config = case.get('oracle_config', {})
        graph = None
        if check in GRAPH_CHECKS:
            from scripts.lint_lib.symbol_graph import build_symbol_graph
            graph = build_symbol_graph(root=root, directories=sorted({f['path'].split('/')[0] for f in files}))
            kwargs = {'execution_only':config.get('options', {}).get('executionOnly', True)} if check == 'public_api_triples' else {}
            if check == 'internal_dead_code' and config.get('options', {}).get('oracleAnalyzer') is False:
                kwargs['extra_root_ids'] = module._build_dynamic_import_root_ids(root, sorted({f['path'].split('/')[0] for f in files}), graph)
            raw = getattr(module, {'internal_dead_code':'analyze_dead_code'}.get(check, 'analyze_'+check))(graph, **kwargs)
        elif check == 'missing_async_pairs':raw = module.analyze_file(path, root)
        elif check in ('sync_async_signatures', 'async_sync_delegation'):raw = module.analyze_file(path)
        elif check in ('data_layer_facade_purity', 'data_layer_compat_aliases'):raw = module.lint_file(path)
        elif check == 'layer_violations':raw = module._check_file_violations(path)
        elif check == 'backend_coupling':
            import ast
            raw = module.BackendCouplingChecker().check_file(ast.parse(files[0]['source']), path, files[0]['source'])
        else:raise ValueError(check)
        return [{**case, 'call':1, 'entry':'source-probe', 'upstream_passed':True, 'production_scope':False,
                 'scope':{'include':['**/*.py'], 'exclude':[]}, 'findings':normalize(check, raw, root, files, graph)}]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'repo', 'inputs', 'output'):parser.add_argument('--'+name, type=Path, required=True)
    args = parser.parse_args()
    source = args.source.resolve()
    sys.path.insert(0, str(source))
    sys.path.insert(1, str(source/'scripts'))
    cases = json.loads(args.inputs.read_text())['cases']
    if any(case['check'] not in BATCH_FIVE for case in cases):raise ValueError('Expected fifth-batch checks')
    events = [event for case in cases for event in record(case, source)]
    output = {'format':'rerail.data-layer-recording/v1', 'commit':PIN, 'checks':sorted({c['check'] for c in cases}),
              'source_mismatches':[], 'pytest_exit_code':0, 'collected_tests':0, 'uncaptured_tests':[],
              'cases':cases, 'events':events, 'loaded_source_files_checked':verify_sources(source, args.repo, PIN)}
    args.output.write_text(json.dumps(output, indent=2)+'\n')
    print(json.dumps({'cases':len(cases), 'observations':len(events), 'positive':sum(bool(e['findings']) for e in events)}))


if __name__ == '__main__':main()
