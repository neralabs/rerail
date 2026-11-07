"""Filesystem and passive DTO transport for native async/sync boundary rules."""
from pathlib import Path
from ast_transport import columns


def select(suite, result, category):
    rows = suite.native('async_sync_boundary', 'category', [],
        dict(categories=[v.category for v in result.violations], category=category))
    if any(row['code'] != 'BS_CATEGORY' for row in rows):
        raise AssertionError('Invalid native boundary category projection')
    return [result.violations[row['parameters']['index']] for row in sorted(rows, key=lambda row: row['parameters']['index'])]


def install(suite, module):
    for name, category in [('sync_wraps_async', 'SYNC_WRAPS_ASYNC'), ('async_wraps_sync', 'ASYNC_WRAPS_SYNC'),
                           ('executor_escape', 'EXECUTOR_ESCAPE')]:
        suite.replace(module.LintResult, name, property(lambda result, category=category: select(suite, result, category)))


def scan(suite, entry, module, args):
    if entry == '_get_call_name':
        rows = suite.native('async_sync_boundary', entry, [], columns(args['node']))
        if len(rows) > 1 or any(row['code'] != 'BS_CALL_NAME' for row in rows):
            raise AssertionError('Invalid native boundary call-name result')
        return rows[0]['parameters']['name'] if rows else None
    if entry in {'_is_allowlisted', '_is_informational'}:
        allowed = sorted(module.ALLOWLIST)
        options = dict(args, allowedFiles=[p for p, _ in allowed], allowedFunctions=[f for _, f in allowed],
                       informationDirs=module.INFORMATIONAL_DIRS)
        rows = suite.native('async_sync_boundary', entry, [], options)
        code = 'BS_ALLOW' if entry == '_is_allowlisted' else 'BS_INFO'
        if len(rows) > 1 or any(row['code'] != code for row in rows):
            raise AssertionError('Invalid native boundary helper result')
        return bool(rows)
    root = Path(args['root'])
    paths = []
    # Filesystem enumeration supplies the source API's requested inventory, including repeated paths.
    # Detection, source gates, allowlists, category projection, and counting execute in the rule.
    for target in args.get('paths') or ['api', 'impl', 'conform']:
        candidate = root / target
        if candidate.is_file():
            paths.append(candidate)
        elif candidate.is_dir():
            paths.extend(sorted(candidate.rglob('*.py')))
    checked = [p.relative_to(root).as_posix() for p in paths]
    files = {}
    for path, logical in zip(paths, checked):
        try:
            source = path.read_text(encoding='utf-8')
        except (OSError, UnicodeError):
            continue
        files[logical] = dict(path=logical, source=source)
    rows = suite.native('async_sync_boundary', entry, list(files.values()), dict(checkedPaths=checked))
    metadata = [r for r in rows if r['code'] == 'BS_COUNT']
    findings = [r for r in rows if r['code'] != 'BS_COUNT']
    if len(metadata) != 1 or any(r['code'] not in {'SYNC_WRAPS_ASYNC', 'ASYNC_WRAPS_SYNC', 'EXECUTOR_ESCAPE'} for r in findings):
        raise AssertionError('Invalid native boundary scan result')
    violations = [module.Violation(file=r['path'], line=r['line'], col=r['parameters']['column'],
        category=r['code'], function_name=r['parameters']['function_name'], call_text=r['parameters']['call_text'],
        is_async_ctx=bool(r['parameters']['async_']))
        for r in sorted(findings, key=lambda r: (r['path'], r['parameters']['ordinal']))]
    return module.LintResult(violations=violations, files_checked=metadata[0]['parameters']['count'])
