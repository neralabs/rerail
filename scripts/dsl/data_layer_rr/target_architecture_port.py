"""Transport source files and typed findings for ServerPlan target architecture."""
import ast
from pathlib import Path


def files_for(sources):
    return [dict(path=Path(path).as_posix(), source=text) for path, text in sources.items()]


def scan(suite, module, arguments, *, entry='scan_sources'):
    check = 'serverplan_target_architecture'
    if entry == '_frozen_slots':
        files = [dict(path='l2070_batching/target-helper.py', source=ast.unparse(arguments['node']) + '\n')]
        rows = suite.native(check, entry, files, dict(requiredShape='false'))
        if len(rows) > 1 or any(r['code'] != 'SPTA_FROZEN' for r in rows):
            raise ValueError('Invalid frozen slots response')
        return bool(rows)
    if entry == '_symbol_names':
        value = arguments['value']
        items = value if isinstance(value, (list, tuple)) else []
        rows = suite.native(check, entry, [], dict(
            inputKind='list' if isinstance(value, list) else 'non-list',
            itemKinds=['str' if isinstance(item, str) else 'non-string' for item in items],
            items=[item if isinstance(item, str) else '' for item in items],
            requiredShape='false'))
        if len(rows) == 1 and rows[0]['code'] == 'SPTA_BAD_SYMBOLS':
            raise AssertionError
        if any(r['code'] != 'SPTA_SYMBOL' for r in rows):
            raise ValueError('Invalid symbol list response')
        return {r['parameters']['value'] for r in rows}
    files = files_for(arguments['sources'])
    shape = str(arguments.get('include_required_shape', True)).lower()
    rows = suite.native(check, entry, files, dict(requiredShape=shape))
    return tuple(
        module.Finding(row['code'], Path(row['path']), row['line'],
                       row['parameters']['name'], row['parameters']['message'])
        for row in sorted(rows, key=lambda r: (r['path'], r['line'], r['code'],
                                               r['parameters']['name'], r['parameters']['order']))
    )
