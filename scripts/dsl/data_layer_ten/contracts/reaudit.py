# SPDX-License-Identifier: Apache-2.0
"""Record fresh production and standalone observations from the pinned linters."""
import argparse
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile


def record(source, output):
    sys.path[:0] = [str(source), str(source / 'scripts')]
    from scripts.lint_lib.fact_engine import FactEngine, _run_plugin_reporter
    from scripts.lint_lib.fact_store import JsonFactStore
    from data_layer_probe_record import verify_sources
    from data_layer_ten.common import PIN
    records = []

    def observe(check, label, path, text, standalone=False):
        module = importlib.import_module('scripts.lint_' + check)
        with tempfile.TemporaryDirectory(prefix='rerail-ten-reaudit-') as directory:
            root = Path(directory)
            target = root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(text.encode())
            previous = Path.cwd()
            os.chdir(root)
            try:
                if standalone:
                    findings = module.find_hnsw_architecture_findings((target,))
                    selected = [path]
                else:
                    plugin = getattr(module, check + '_plugin')(root)
                    engine = FactEngine(root=root, fact_store=JsonFactStore(root / '.facts'),
                        extractors=getattr(module, check + '_fact_extractors')(root), enable_fact_cache=False)
                    selected = sorted(path.relative_to(root).as_posix() for path in engine.plan((plugin,)).files)
                    findings = _run_plugin_reporter(plugin, engine.materialize((plugin,), max_workers=1)).violations
                expected = [dict(path=row.path.relative_to(root).as_posix() if row.path.is_absolute() else row.path.as_posix(),
                                 line=row.line, code=row.code, message=row.message) for row in findings]
            finally:
                os.chdir(previous)
        records.append(dict(test=check + ':' + label, check=check,
            entry='find_hnsw_architecture_findings' if standalone else 'all', parameters={},
            production=not standalone, files=[dict(path=path, source=text)], expected=expected, selected=selected))

    positive = 'SqlHnswBeamLayerSearch()\n'
    for label, path in [('root-file', 'vector_types.py'), ('root-directory', 'vector_types.py/probe.py'),
                        ('common', 'impl/common/probe.py'), ('vector', 'l5200_vector_index/probe.py'),
                        ('test-directory', 'impl/common/tests/probe.py'), ('ignored-directory', 'impl/common/tmp/probe.py')]:
        observe('hnsw_architecture', label, path, positive)
    observe('hnsw_architecture', 'clean', 'impl/common/probe.py', 'ordinary()\n')
    observe('hnsw_architecture', 'standalone-root-positive', 'vector_types.py', positive, standalone=True)
    observe('hnsw_architecture', 'standalone-root-clean', 'vector_types.py', 'ordinary()\n', standalone=True)
    from data_layer_ten.contracts.adversarial import record as adversarial
    records.extend(adversarial())
    output.write_text(json.dumps(dict(format='rerail.data-layer-batch-ten-reaudit/v1', sourcePin=PIN,
        sourceFilesChecked=verify_sources(source, source, PIN), cases={}, sources={}, records=records), indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    record(args.source.resolve(), args.output)
