"""Transfer source buffers and public import DTOs; native rules make every decision."""
from pathlib import Path


def scan(suite, entry, module, arguments):
    path = Path(arguments['file_path'])
    files = []
    try:
        source = path.read_text()
    except (OSError, UnicodeError):
        pass
    else:
        files.append(dict(path=path.name, source=source))
    root = Path(module.__file__).resolve().parents[1]
    try:
        scope_path = path.relative_to(root)
    except ValueError:
        scope_path = path
    options = dict(pathParts=list(scope_path.parts))
    if entry == 'check_file_content':
        options['providedSource'] = arguments['source']
    rows = suite.native('no_api_root_imports', entry, files, options)
    if any(row['code'] != 'NARI001' for row in rows):
        raise AssertionError('Unexpected native API-import diagnostic')
    rows.sort(key=lambda r: (r['parameters']['ordinal'], r['parameters'].get('piece', 0),
                            r['parameters']['pattern'], r['parameters']['occurrence']))
    return [module.Violation(file_path=path, line_number=r['line'], message=r['message']) for r in rows]
