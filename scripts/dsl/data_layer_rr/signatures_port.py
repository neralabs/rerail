"""Transfer signature mismatch DTOs; all annotation and function selection runs natively."""
from pathlib import Path


def legacy_message(mismatch):
    """Display a passive DTO without calling the original module's methods."""
    sync, async_ = mismatch.sync_func, mismatch.async_func
    return (f"{mismatch.file}:\n"
        f"  SYNC  line {sync.line}: {sync.name}({sync.first_param_name}: {sync.first_param_type})\n"
        f"  ASYNC line {async_.line}: {async_.name}({async_.first_param_name}: {async_.first_param_type})\n"
        f"  -> Async version should take {sync.first_param_type}, not {async_.first_param_type}")


def scan(suite, entry, module, args):
    path = Path(args['file_path'])
    try:
        source = path.read_text()
    except (OSError, UnicodeDecodeError):
        return []
    project = Path(module.__file__).parent.parent
    logical = path.relative_to(project).as_posix() if path.is_relative_to(project) else path.name
    rows = suite.native('sync_async_signatures', entry, [dict(path=logical, source=source)])
    if any(row['code'] != 'sync_async_signature_mismatch' for row in rows):
        raise AssertionError('Unexpected signature mismatch diagnostic')
    result = []
    for row in sorted(rows, key=lambda r: r['parameters']['ordinal']):
        value = row['parameters']
        def signature(prefix):
            return module.FunctionSignature(name=value[prefix + '_name'],
                is_async=bool(value[prefix + '_is_async']), first_param_name=value[prefix + '_param'],
                first_param_type=value[prefix + '_type'], line=value[prefix + '_line'], file=path)
        result.append(module.SignatureMismatch(signature('sync'), signature('async'), path))
    return result
