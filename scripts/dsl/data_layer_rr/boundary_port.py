"""Filesystem and passive DTO transport for native async/sync boundary rules."""
from pathlib import Path
from ast_transport import columns


def comparison_value(value):
    """Preserve complete values while comparing the source's nondeterministic file-completion groups.

    fact_engine.py yields as_completed futures. scan_tree appends each complete file bundle.
    Repeated unchanged source runs demonstrate both cross-file orders. In-file order stays exact.
    The source and native reports retain their original values; this copy is only for comparison.
    """
    import copy
    result = copy.deepcopy(value)
    if 'result' not in result:
        return result
    owner = result['result']
    if owner.get('type') != 'scripts.lint_async_sync_boundary.LintResult':
        raise ValueError('Invalid boundary result type')
    groups = {}
    previous = None
    for row in owner['fields']['violations']:
        path = row['fields']['file']
        if path != previous and path in groups:
            raise ValueError('Boundary file groups are interleaved')
        groups.setdefault(path, []).append(row)
        previous = path
    owner['fields']['violations'] = [row for path in sorted(groups) for row in groups[path]]
    return result


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
    selected = suite.native('async_sync_boundary','_bs_select',[],dict(bsCandidatePaths=checked,bsCandidateIsFile=[str(int(p.is_file())) for p in paths]))
    if any(r['code']!='BS_FILE' for r in selected):raise AssertionError('Invalid native boundary file selection')
    files, owners, error = [], {}, None
    for row in sorted(selected,key=lambda r:r['parameters']['ordinal']):
        i=row['parameters']['ordinal'];path=paths[i];logical=checked[i]
        if row['parameters']['path']!=logical:raise AssertionError('Native boundary selected a different input path')
        try:
            source = path.read_text(encoding='utf-8')
        except (OSError, UnicodeError) as caught:
            error = caught
            break
        key=f'api/_bs_buffers/{i}.py';files.append(dict(path=key,source=source));owners[key]=logical
    rows = suite.native('async_sync_boundary', entry, files,
                        dict(checkedPaths=checked, hasReadError=str(error is not None).lower()))
    failures = [r for r in rows if r['code'] == 'BS_READ_ERROR']
    if failures:
        if len(failures) != 1 or error is None or any(r['code'] not in {'BS_READ_ERROR', 'BS_COUNT'} for r in rows):
            raise AssertionError('Invalid native boundary read failure')
        raise error
    metadata = [r for r in rows if r['code'] == 'BS_COUNT']
    findings = [r for r in rows if r['code'] != 'BS_COUNT']
    if len(metadata) != 1 or any(r['code'] not in {'SYNC_WRAPS_ASYNC', 'ASYNC_WRAPS_SYNC', 'EXECUTOR_ESCAPE'} for r in findings):
        raise AssertionError('Invalid native boundary scan result')
    violations = [module.Violation(file=owners[r['path']], line=r['line'], col=r['parameters']['column'],
        category=r['code'], function_name=r['parameters']['function_name'], call_text=r['parameters']['call_text'],
        is_async_ctx=bool(r['parameters']['async_']))
        for r in sorted(findings, key=lambda r: (owners[r['path']], r['parameters']['ordinal']))]
    return module.LintResult(violations=violations, files_checked=metadata[0]['parameters']['count'])
