"""Transfer facade alias APIs to native decisions and passive result storage."""
from pathlib import Path


def snapshot(module, entry, args, kwargs, item, source):
    supplied = Path(args[0] if args else kwargs['path'])
    path = supplied.resolve()
    root = item.funcargs.get('tmp_path', path.parent)
    if supplied.is_absolute() and path.is_relative_to(source):
        root = source
    content = (args[1] if len(args) > 1 else kwargs.get('source')) if entry == 'lint_file_content' else path.read_text() if path.is_file() else None
    logical = supplied.as_posix() if not supplied.is_absolute() else path.relative_to(root).as_posix() if path.is_relative_to(root) else path.name
    files = [dict(path=logical, source=content)] if content is not None else []
    return files, dict(include=['**/*.py'], exclude=[]), root


def scan(suite, entry, module, args):
    check = 'data_layer_compat_aliases'
    if entry == '_is_public_name':
        rows = suite.native(check, entry, [], dict(name=args['name']))
        if len(rows) > 1 or any(r['code'] != 'ALIAS_PUBLIC' for r in rows):
            raise AssertionError('Invalid native public-name predicate')
        return bool(rows)
    if entry == '_leaf_name':
        # The original assertion supplies an AST DTO. Transport its fields; .rr selects the kinds.
        node = args['node']
        options = dict(nodeKind=type(node).__name__)
        for field in ['id', 'attr']:
            if isinstance(getattr(node, field, None), str):
                options['nodeName'] = getattr(node, field)
        rows = suite.native(check, entry, [], options)
        if len(rows) > 1 or any(r['code'] != 'ALIAS_LEAF' for r in rows):
            raise AssertionError('Invalid native leaf-name projection')
        return rows[0]['parameters']['result'] if rows else None
    targets = list(map(Path, args['targets'])) if entry == 'lint_targets' else [Path(args['path'])]
    paths = []
    for target in targets:
        if target.is_file():
            paths.append(target)
        else:
            paths.extend(sorted(p for p in target.glob('*.py') if p.is_file()))
    originals, files = {}, []
    for index, path in enumerate(paths):
        name = f'input/{index}/{path.name}'
        originals[name] = path
        try:
            content = path.read_text()
        except (OSError, UnicodeError):
            continue
        files.append(dict(path=name, source=content))
    rows = suite.native(check, entry, files, dict(apiInputDirs=['input']))
    if any(r['code'] not in {'DLA001', 'DLA002'} for r in rows):
        raise AssertionError('Unexpected facade alias diagnostic')
    rows.sort(key=lambda r: (originals[r['path']].as_posix(), r['line'], r['code']))
    violations = [module.Violation(originals[r['path']], r['line'], r['code'], r['message']) for r in rows]
    return module.LintResults(violations=violations, files_checked=len(paths)) if entry == 'lint_targets' else violations
