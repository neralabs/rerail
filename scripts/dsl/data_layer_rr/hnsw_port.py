"""Transfer the HNSW file API with complete paths, tuples, and findings."""
from pathlib import Path


def scan(suite, entry, module, args):
    originals, files, errors = {}, [], {}
    external = []
    cwd = Path.cwd()
    for path in args['paths']:
        path = Path(path)
        name = path.relative_to(cwd).as_posix() if path.is_absolute() and path.is_relative_to(cwd) else path.as_posix().lstrip('/')
        originals[name] = path
        if path.is_absolute() and not path.is_relative_to(cwd):
            external.append(name)
        try:
            source = path.read_text(encoding='utf-8')
        except (OSError, UnicodeError) as error:
            # The native inventory rule decides whether a forbidden path bypasses file IO.
            errors[name], source = error, ''
        files.append(dict(path=name, source=source))
    rows = suite.native('hnsw_architecture', entry, files, dict(cwdExternalPaths=external))
    if any(r['code'] not in {'HNSW001', 'HNSW002', 'HNSW003', 'HNSW004', 'HNSW005', 'HNSW006', 'HNSW007'} for r in rows):
        raise AssertionError('Unexpected HNSW diagnostic')
    inventory = {r['path'] for r in rows if r['code'] == 'HNSW006'}
    for name, error in errors.items():
        if name not in inventory:
            raise error
    rows.sort(key=lambda r: (str(originals[r['path']]), r['line'], r['code'], r['message']))
    return tuple(module.HnswArchitectureFinding(originals[r['path']], r['line'], r['code'], r['message']) for r in rows)
