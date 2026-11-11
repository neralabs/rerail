"""Transport connection/transaction file inputs and passive Violation DTOs."""
from pathlib import Path


def scan(suite, entry, module, args):
    path = Path(args['file_path'])
    decode_error = None
    try:
        source = path.read_text(encoding='utf-8')
    except UnicodeDecodeError as error:
        decode_error, source = error, ''
    except OSError:
        source = ''
    root = Path(args.get('root') or Path.cwd())
    absolute = path if path.is_absolute() else Path.cwd() / path
    logical = absolute.relative_to(root).as_posix() if absolute.is_relative_to(root) else absolute.as_posix().lstrip('/')
    # A direct file API does not inherit production discovery roots or exclusions.
    # Preserve the caller path as data; the rule applies its explicit-file contract.
    rows = suite.native('conn_or_txn_union', entry, [dict(path='impl/input.py', source=source)],
        dict(apiSourcePath=logical,decodeError=str(decode_error is not None).lower()))
    if any(row['code'] == 'CUT_DECODE_ERROR' for row in rows):
        if len(rows) != 1 or decode_error is None:
            raise AssertionError('Invalid native decoding error')
        raise decode_error
    if any(row['code'] not in {'UNION', 'SEPARATE_PARAMS'} for row in rows):
        raise AssertionError('Invalid connection/transaction diagnostic')
    return [module.Violation(file_path=absolute, line_number=row['line'],
        function_name=row['parameters']['function_name'], violation_type=row['code'], details=row['parameters']['details'])
        for row in sorted(rows, key=lambda row: row['parameters']['ordinal'])]
