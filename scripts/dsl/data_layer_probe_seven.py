#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Evaluate seventh-batch supplemental inputs with the unchanged pinned oracle."""
from __future__ import annotations
import argparse
import ast
import importlib
import json
from pathlib import Path
import sys
import tempfile
from contextlib import contextmanager
from data_layer_batch_seven import CHECKERS, MODULES, normalize
from data_layer_probe_record import verify_sources

PIN = '2b1504adc4291daaf080b2e932962b134da1a69a'


@contextmanager
def post_op_repository(module, root, options):
    """Run unchanged functions with the file constants of the fixture repository."""
    paths = {
        'POST_OP_SOURCE': 'l2070_batching/post_op.py',
        'POST_OP_SPECS_SOURCE': 'l2070_batching/post_op_specs.py',
        'POST_OP_DOMAIN_DECODERS_SOURCE': 'l2070_batching/post_op_domain_decoders.py',
        'POST_OP_RUNTIME_ALLOWLIST_SOURCE': 'l2070_batching/post_op_allowlist.py',
    }
    updates = {'REPO_ROOT': root, **{key: root / path for key, path in paths.items()}}
    updates['POST_OP_SURFACE_FILES'] = tuple(updates[key] for key in ('POST_OP_SOURCE', 'POST_OP_SPECS_SOURCE', 'POST_OP_DOMAIN_DECODERS_SOURCE'))
    for option, name in [('fatesSignature', 'POST_OP_FATES_SIGNATURE'), ('surfaceSignature', 'POST_OP_SURFACE_SIGNATURE')]:
        if option in options: updates[name] = options[option]
    if 'fateOverrides' in options:
        updates['POST_OP_FATE_OVERRIDES'] = {
            symbol: module.PostOpFate(fate) for symbol, fate in (entry.split('=', 1) for entry in options['fateOverrides'])
        }
    saved = {key: getattr(module, key) for key in updates}
    try:
        for key, value in updates.items(): setattr(module, key, value)
        yield
    finally:
        for key, value in saved.items(): setattr(module, key, value)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'repo', 'inputs', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    source = args.source.resolve()
    sys.path[:0] = [str(source), str(source / 'scripts')]
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore
    events = []
    for case in json.loads(args.inputs.read_text())['cases']:
        check = case['check']; module = importlib.import_module(MODULES[check]); files = case['files']
        with tempfile.TemporaryDirectory(prefix='rerail-dl-b7-oracle-') as temporary:
            root = Path(temporary)
            for file in files:
                path = root / file['path']; path.parent.mkdir(parents=True, exist_ok=True); path.write_text(file['source'])
            production = case.get('production_scope', False)
            if production and check == 'post_op_side_effects':
                with post_op_repository(module, root, case.get('options', {})):
                    plugin = module.post_op_side_effects_plugin(root)
                    engine = FactEngine(root=root, fact_store=JsonFactStore(root / '.facts'),
                                       extractors=module.post_op_side_effects_fact_extractors(root), enable_fact_cache=False)
                    raw = plugin.reporter(engine.materialize((plugin,), max_workers=1))
                entry = 'plugin.reporter'
            elif production:
                factory = getattr(module, check + '_plugin')
                plugin = factory(root)
                engine = FactEngine(root=root, fact_store=JsonFactStore(root / '.facts'),
                                   extractors=getattr(module, check + '_fact_extractors')(root), enable_fact_cache=False)
                raw = plugin.reporter(engine.materialize((plugin,), max_workers=1))
                entry = 'plugin.reporter'
            elif check in CHECKERS:
                file = files[0]
                raw = getattr(module, CHECKERS[check])().check_file(ast.parse(file['source']), Path(file['path']), file['source'])
                entry = 'check_file'
            elif check == 'executor_publication_repairs':
                raw = module.violations(root / files[0]['path']); entry = 'violations'
            elif check == 'revision_ancestry_invariants':
                raw = module.violations(tuple(root / file['path'] for file in files)); entry = 'violations'
            elif check == 'history_serverplan_projection':
                raw = module.check_source(module.ROOT / files[0]['path'], files[0]['source']); entry = 'check_source'
            elif check == 'post_op_no_internal_dispatch':
                path = module.POST_OP_SOURCE if case.get('post_op_source') else Path(files[0]['path'])
                raw = module.collect_violations_for_source(path, files[0]['source']); entry = 'collect_violations_for_source'
            elif check == 'versioned_merged_identity':
                raw = module.collect_violations(root, expected_callers=frozenset(case.get('expected_callers', module.EXPECTED_CALLERS)))
                entry = 'collect_violations'
            else:
                raise ValueError('No supplemental adapter for ' + check)
            def logical(path):
                path = Path(path)
                for base in (root, getattr(module, 'ROOT', getattr(module, 'REPO_ROOT', source))):
                    if path.is_absolute() and path.is_relative_to(base): return path.relative_to(base).as_posix()
                return path.as_posix()
            findings = normalize(check, raw, files, logical)
            options = {'oracleAnalyzer': not production}
            if check == 'versioned_merged_identity':
                options = {'expectedCallers': sorted(case.get('expected_callers', module.EXPECTED_CALLERS))}
            elif check == 'post_op_side_effects':
                options = {'entryPoint': entry, **case.get('options', {})}
            events.append({'test': 'probe:batch7:' + case['id'], 'check': check, 'call': 1, 'entry': entry,
                           'upstream_passed': True, 'production_scope': production,
                           'scope': {'include': ['**/*.py'], 'exclude': []}, 'files': files,
                           'oracle_config': {'options': options}, 'findings': findings})
            if production and check == 'post_op_side_effects':
                events[-1]['oracle_context'] = 'File constants point to the temporary fixture repository; implementations and signing keys are unchanged.'
    report = {'format': 'rerail.data-layer-recording/v1', 'commit': PIN, 'checks': sorted({e['check'] for e in events}),
              'source_mismatches': [], 'pytest_exit_code': 0, 'collected_tests': 0, 'uncaptured_tests': [],
              'loaded_source_files_checked': verify_sources(source, args.repo, PIN), 'events': events}
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'observations': len(events), 'positive': sum(bool(e['findings']) for e in events),
                      'clean': sum(not e['findings'] for e in events)}))


if __name__ == '__main__': main()
