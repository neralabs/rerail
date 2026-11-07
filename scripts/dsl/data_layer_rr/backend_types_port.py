"""Transport backend scanner inputs and DTOs; native rules select paths and findings."""
from pathlib import Path


def scan(suite, entry, module, arguments):
    if entry == 'should_skip_file':
        rows = suite.native('backend_type_strings', entry, [], dict(path=str(arguments['path'])))
        if len(rows) > 1 or any(row['code'] != 'BTS_SKIP' for row in rows):
            raise AssertionError('Invalid native backend skip predicate')
        return bool(rows)
    if entry == 'find_python_files':
        candidates = []
        for raw in arguments['paths']:
            path = Path(raw)
            if path.is_file():
                candidates.append(str(path))
            elif path.is_dir():
                # Transport the directory inventory. Native suffix selection and set semantics decide the result.
                candidates.extend(str(item) for item in path.rglob('*'))
        rows = suite.native('backend_type_strings', entry, [], dict(candidates=candidates))
        if any(row['code'] != 'BTS_FILE' for row in rows):
            raise AssertionError('Invalid native backend file inventory')
        return sorted({Path(row['parameters']['result']) for row in rows})
    path = Path(arguments['path'])
    options = dict(sourcePath=str(path))
    predicate = module.should_skip_file
    if getattr(predicate, '__module__', None) != module.__name__:
        # A caller-supplied predicate is an API input. The original source predicate never executes here.
        options['skipOverride'] = str(bool(predicate(path))).lower()
    files = []
    try:
        content = arguments['content'] if entry == 'check_file_content' else path.read_text()
    except Exception:
        pass  # The source file API catches all read failures and returns no findings.
    else:
        files.append(dict(path='input.py', source=content))
    rows = suite.native('backend_type_strings', entry, files, options)
    if any(row['code'] not in {'BT000', 'BT001', 'BT002', 'BT003', 'BT004', 'BT005', 'BTS_SKIPPED'} for row in rows):
        raise AssertionError('Unexpected native backend type diagnostic')
    rows = [row for row in rows if row['code'] != 'BTS_SKIPPED']
    rows.sort(key=lambda row: (row['line'], row['parameters']['pattern'], row['parameters']['occurrence']))
    return [module.Violation(file=path, line=row['line'], column=row['parameters']['column'],
        message=row['message'], code=row['code'], snippet=row['parameters']['snippet']) for row in rows]


def install(suite, module):
    """Back unchanged constant assertions with one native configuration batch."""
    import re
    rows = suite.native('backend_type_strings', 'configuration', [])
    if not rows or any(row['code'] != 'BTS_CONFIG' for row in rows):
        raise AssertionError('Missing native backend type configuration')
    tables = {}
    for row in sorted(rows, key=lambda row: (row['parameters']['key'], row['parameters']['ordinal'])):
        values = row['parameters']
        tables.setdefault(values['key'], []).append(values['value'])
    suite.replace(module, 'BACKEND_STRINGS', frozenset(tables['BACKEND_STRINGS']))
    suite.replace(module, 'SKIP_PATTERNS', tables['SKIP_PATTERNS'])
    suite.replace(module, 'COMPARISON_PATTERNS', [re.compile(value) for value in tables['COMPARISON_PATTERNS']])
    if not hasattr(suite, 'configuration_bindings'):
        suite.configuration_bindings = {}
    suite.configuration_bindings['backend_type_strings'] = dict(nativeCall=len(suite.calls) - 1,
        names=['BACKEND_STRINGS', 'SKIP_PATTERNS', 'COMPARISON_PATTERNS'])
