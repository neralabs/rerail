"""Compare legacy text scans, cyclic call graphs, and repository errors."""
import gzip
import json
from pathlib import Path

from dto import canonical
from migration import fixtures_path

CHECK = 'legacy_execution_surfaces'


def cases(_module):
    path = fixtures_path('host', 'data-layer-port', 'rr-missing-four-original-recording.json.gz')
    with gzip.open(path, 'rt', encoding='utf-8') as stream:
        recording = json.load(stream)
    text_cases = [
        dict(kind='text', id=row['test'], **row['files'][0])
        for row in recording['events']
        if row['check'] == CHECK and row['entry'] == 'scan_sql_post_fetch_filter_content'
    ]
    filter_token = 'filter_query_results_async'
    return text_cases + [
        dict(kind='text', id='splitlines', path='docs/notes.rst',
             source=f'préface\u2028\u2029{filter_token}\r\n{filter_token}\v# end\n'),
        dict(kind='text', id='history-exclusion', path='docs/data_history/notes.md', source=filter_token),
        dict(kind='ast', id='memoized-cycle', path='l9750_graphql/executor.py', source=(
            'def a():\n    b()\n    c()\n'
            'def b():\n    a()\n'
            'def c():\n    lazy.resolve()\n'
        )),
        dict(kind='repo', id='inventory-and-text', files={
            'docs/policy.rst': f'préface\u2028\u2029{filter_token}\r\n{filter_token}\v# end\n',
            'other/export_cursor.py': 'pass\n',
            'impl/common/l5200_sql_query/_pagination_leaf.py': 'pass\n',
            'l9750_graphql/terminal.py': 'def read(txn):\n    return txn.select_one()\n',
            'l9750_graphql/broken.py': 'def broken(:\n',
            '.pytest_cache/noisy/export_cursor.py': 'pass\n',
        }),
        dict(kind='repo', id='invalid-utf8', files={}, binary={
            'l9750_graphql/invalid.py': 'ff',
            'l4000_permissions/invalid.py': 'ff',
            'docs/invalid.rst': 'f09f',
        }),
    ]


def exercise(module, case, temporary):
    kind = case['kind']
    if kind == 'text':
        return canonical(module.scan_sql_post_fetch_filter_content(case['path'], case['source']))
    if kind == 'ast':
        return canonical(module.scan_file_content(case['path'], case['source']))
    root = temporary / 'repo'
    root.mkdir()
    for name, text in case['files'].items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode('utf-8'))
    for name, raw in case.get('binary', {}).items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(bytes.fromhex(raw))
    return canonical(module.scan_repo(root), root)


def install(backend, module, detectors=False):
    from legacy_execution_port import scan

    for name in ('scan_file_content', 'scan_sql_post_fetch_filter_content'):
        def scan_content(rel_path, source, tree=None, *, entry=name):
            return scan(backend, entry, module, dict(rel_path=rel_path, source=source))
        backend.replace(module, name, scan_content)

    def scan_repo(root):
        return scan(backend, 'scan_repo', module, dict(root=root))
    backend.replace(module, 'scan_repo', scan_repo)


if __name__ == '__main__':
    from contract_runner import main

    main(CHECK, 'rerail.legacy-execution-scanner-contract/v1', cases, exercise, install,
         ['legacy_execution_scanner_contract.py', 'legacy_execution_port.py'])
