# SPDX-License-Identifier: Apache-2.0
"""Record actual configured reporter results for persistent workspace transitions."""
import argparse
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile


def observe(check, files):
    from scripts.lint_lib.fact_engine import FactEngine, _run_plugin_reporter
    from scripts.lint_lib.fact_store import JsonFactStore
    module = importlib.import_module('scripts.lint_' + check)
    with tempfile.TemporaryDirectory(prefix='rerail-lifecycle-') as directory:
        root = Path(directory)
        for file in files:
            path = root / file['path']
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(file['source'].encode())
        oldroot = getattr(module, 'ROOT', getattr(module, 'REPO_ROOT', root))
        changed = []
        for name, value in list(vars(module).items()):
            if isinstance(value, Path) and value.is_absolute() and value.is_relative_to(oldroot):
                changed.append((name, value))
                setattr(module, name, root / value.relative_to(oldroot))
        previous = Path.cwd()
        os.chdir(root)
        try:
            if check in {'bounded_composed_graph_paths', 'matching_architecture'}:
                findings = module.collect_violations(root) if check == 'bounded_composed_graph_paths' else module.lint(root)
            else:
                plugin = getattr(module, check + '_plugin')(root)
                engine = FactEngine(root=root, fact_store=JsonFactStore(root / '.facts'), extractors=getattr(module, check + '_fact_extractors')(root))
                findings = _run_plugin_reporter(plugin, engine.materialize((plugin,), max_workers=1)).violations
            rows = []
            for finding in findings:
                path = Path(getattr(finding, 'path', getattr(finding, 'file_path', '')))
                rows.append(dict(path=path.relative_to(root).as_posix() if path.is_absolute() else path.as_posix(), line=finding.line, code=finding.code, message=finding.message))
            return sorted(rows, key=lambda row: (row['path'], row['line'], row['code'], row['message']))
        finally:
            os.chdir(previous)
            for name, value in changed:
                setattr(module, name, value)


def record(source, output):
    from data_layer_ten.common import CHECKS, PIN
    from data_layer_ten.evidence import read_report
    from data_layer_nine.evidence import expand
    from data_layer_rule_audit import fixtures_path
    from data_layer_probe_record import verify_sources
    sys.path[:0] = [str(source), str(source / 'scripts')]
    configured = expand(read_report(fixtures_path('host/data-layer-port/batch-10-configured-defaults.json.gz')))
    cases = []
    for check in CHECKS:
        clean = next(row for row in configured['records'] if row['test'] == check + ':clean')
        positive = next(row for row in configured['records'] if row['test'] == check + ':positive')
        states = []
        for name, files in [('clean', clean['files']), ('positive', positive['files']), ('restored', clean['files']), ('removed', []), ('readded', positive['files'])]:
            states.append(dict(name=name, files=files, expected=observe(check, files)))
        assert not states[0]['expected'] and states[1]['expected'], check
        cases.append(dict(check=check, states=states))
    output.write_text(json.dumps(dict(format='rerail.data-layer-lifecycle/v1', sourcePin=PIN, sourceFilesChecked=verify_sources(source, source, PIN), cases=cases), indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    record(args.source.resolve(), args.output)
