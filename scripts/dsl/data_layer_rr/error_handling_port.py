"""Transport file inputs and error-handling tuples. Native rules own the gates and decisions."""
from pathlib import Path


def snapshot(module, entry, args, kwargs):
    path = Path(args[0] if args else kwargs['path'])
    root = Path(args[1] if len(args) > 1 else kwargs['root'])
    actual = path if path.is_absolute() else root / path
    files = []
    try:
        source = actual.read_text()
    except (OSError, UnicodeError):
        pass
    else:
        files.append(dict(path=actual.relative_to(root).as_posix(), source=source))
    return files, dict(include=['**/*.py'], exclude=[]), root


def normalize(result, root):
    return [dict(path=path.relative_to(root).as_posix(), line=line, column=None, code='EH001', message=message)
        for path, line, message in result]


def scan(suite, entry, module, arguments):
    root, path = Path(arguments['root']), Path(arguments['path'])
    actual = path if path.is_absolute() else root / path
    options = dict(filename=actual.name, providedSource=arguments['source'], apiInputDirs=['input'])
    try:
        relative = actual.relative_to(root)
    except ValueError:
        relative = actual
        options['outsideRoot'] = 'true'
    options.update(relativePath=relative.as_posix(), pathParts=list(relative.parts))
    options['isFile'] = str(actual.is_file()).lower()
    read_error = None
    try:
        source = actual.read_text(encoding='utf-8')
    except (OSError, UnicodeError) as error:
        read_error = error
        options['readError'] = 'true'
        source = ''
    rows = suite.native('error_handling', entry, [dict(path='input/source.py', source=source)], options)
    failures = [row for row in rows if row['code'] == 'EH_READ_ERROR']
    if failures:
        if len(failures) != 1 or len(rows) != 1 or read_error is None:
            raise AssertionError('Invalid native error-handling read failure')
        raise read_error
    if any(row['code'] != 'EH001' for row in rows):
        raise AssertionError('Unexpected native error-handling diagnostic')
    rows.sort(key=lambda row: (row['parameters']['ordinal'], row['parameters']['witness']))
    return [(actual, row['line'], row['message']) for row in rows]
