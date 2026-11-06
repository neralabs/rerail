# SPDX-License-Identifier: Apache-2.0
"""Record bootstrap inventory stress cases with the original detector."""
import argparse
from dataclasses import asdict
import hashlib
import gzip
import json
from pathlib import Path
import sys


def record(source, output):
    from data_layer_probe_record import verify_sources
    from data_layer_ten.common import PIN
    sys.path[:0] = [str(source), str(source / 'scripts')]
    from scripts import lint_bootstrap_engine_invariants as original
    sources, records = {}, []
    inventory = [dict(path=path.relative_to(source).as_posix(), source=path.read_text())
                 for path in original._scan_paths()]
    for name, helper in [('positive', 'table_exists()\n'), ('clean', 'ordinary()\n')]:
        files = [{**file, 'source': file['source'] + ('\n' + helper if file['path'] == 'data_layer_testing/helpers/bootstrap.py' else '')}
                 for file in inventory]
        expected = []
        try:
            for file in files:
                expected.extend({**asdict(row), 'path': row.path.relative_to(source).as_posix()}
                                for row in original._collect_file_violations(source / file['path'], source=file['source']))
            expected.extend({**asdict(row), 'path': row.path.relative_to(source).as_posix()}
                            for row in original._collect_primitive_install_coverage_violations())
        finally:
            original._walk.cache_clear()
        assert bool(expected) == (name == 'positive'), (name, expected)
        inputs = []
        for file in files:
            digest = hashlib.sha256(file['source'].encode()).hexdigest()
            sources[digest] = file['source']
            inputs.append(dict(path=file['path'], sourceSha256=digest))
        records.append(dict(test='bootstrap-inventory-' + name, check='bootstrap_engine_invariants',
                            entry='all', files=inputs, parameters={}, expected=expected))
    data = json.dumps(dict(format='rerail.data-layer-bootstrap-inventory/v1', sourcePin=PIN,
        cases={}, sources=sources, records=records,
        loaded_source_files_checked=verify_sources(source, source, PIN)), indent=2) + '\n'
    output.write_bytes(gzip.compress(data.encode(), mtime=0))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    record(args.source.resolve(), args.output)
