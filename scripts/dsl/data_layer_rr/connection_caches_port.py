"""Transport cache-scanner inputs and tuples. The rule owns all pattern and allowlist decisions."""
from pathlib import Path


def paths_for(module, root, directories=None):
    root = Path(root)
    paths = []
    for directory in module.SCAN_DIRECTORIES if directories is None else directories:
        target = root / directory
        if target.exists():
            paths.extend(target.rglob('*.py'))
    return paths


def snapshot(module, entry, args, kwargs):
    if entry == 'find_violations':
        root = Path(args[0] if args else kwargs['root'])
        directories = args[1] if len(args) > 1 else kwargs.get('directories')
        paths = paths_for(module, root, directories)
    else:
        path = Path(args[0] if args else kwargs['file_path'])
        root = Path(args[1] if len(args) > 1 else kwargs['root'])
        paths = [path]
    files = []
    for path in paths:
        try:
            content = (args[2] if len(args) > 2 else kwargs['source']) if entry == 'scan_file_content' else path.read_text(encoding='utf-8')
        except (OSError, UnicodeDecodeError):
            continue
        files.append(dict(path=path.relative_to(root).as_posix(), source=content))
    return files, dict(include=['**/*.py'], exclude=[]), root


def normalize(result, root):
    return [dict(path=path.relative_to(root).as_posix(), line=line, column=None, code='CC001', message=description)
        for path, line, _, description in result]


def scan(suite, entry, module, args):
    if entry == '_is_allowed':
        rows = suite.native('connection_caches', entry, [], dict(path=args['file_rel'], line=args['line']))
        if len(rows) > 1 or any(row['code'] != 'CC_ALLOWED' for row in rows):
            raise AssertionError('Unexpected native cache allowlist result')
        return bool(rows)
    root = Path(args['root'])
    paths = paths_for(module, root, args.get('directories')) if entry == 'find_violations' else [Path(args['file_path'])]
    files, originals = [], {}
    for index, path in enumerate(paths):
        if not path.is_relative_to(root):
            rows = suite.native('connection_caches', 'skip_path', [], dict(pathParts=list(path.parts)))
            if rows:
                if any(row['code'] != 'CC_SKIP' for row in rows):
                    raise AssertionError('Unexpected native cache path gate')
                continue
            path.relative_to(root)  # Preserve the input-domain error from the source API.
        try:
            source = args['source'] if entry == 'scan_file_content' else path.read_text(encoding='utf-8')
        except (OSError, UnicodeDecodeError):
            continue
        logical = f'input/{index}/' + path.relative_to(root).as_posix()
        originals[logical] = path
        files.append(dict(path=logical, source=source))
    rows = suite.native('connection_caches', entry, files,
                        dict(apiInputDirs=['input'], rootParts=list(root.parts)))
    if any(row['code'] != 'CC001' for row in rows):
        raise AssertionError('Unexpected native connection cache diagnostic')
    order = {file['path']: index for index, file in enumerate(files)}
    rows.sort(key=lambda row: (order[row['path']], row['line'], row['parameters']['ordinal']))
    return [(originals[row['path']], row['line'], row['parameters']['snippet'], row['parameters']['description']) for row in rows]


def install(suite, module):
    """Use native configuration as passive carriers for the original metadata assertions."""
    import re
    rows = suite.native('connection_caches', 'configuration', [])
    if not rows or any(row['code'] != 'CC_CONFIG' for row in rows):
        raise AssertionError('Missing native cache configuration')
    tables = {}
    for row in sorted(rows, key=lambda row: (row['parameters']['key'], row['parameters']['ordinal'])):
        value = row['parameters']
        tables.setdefault(value['key'], []).append(value['value'])
    patterns = [re.compile(text) for text in tables['patterns']]
    suite.replace(module, 'ALL_PATTERNS', list(zip(patterns, tables['descriptions'])))
    for name, pattern in zip(['BRANCH_REVISION_CACHE_PATTERN', 'CONNECTION_FILE_CACHE_DICT', 'MODULE_BRANCH_REVISION_DICT', 'SELF_BRANCH_REVISION_DICT'], patterns):
        suite.replace(module, name, pattern)
    suite.replace(module, 'SCAN_DIRECTORIES', tables['directories'])
    suite.replace(module, 'SKIP_PARTS', set(tables['skip']))
    suite.replace(module, 'ALLOWED_CACHES', list(zip(tables['allowed_files'], tables['allowed_substrings'], tables['allowed_reasons'])))
