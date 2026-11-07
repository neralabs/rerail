"""Transfer backend-coupling inputs and DTOs. Detection runs only in the native rule."""
from pathlib import Path


def snapshot(path, source):
    path = Path(path)
    root = None
    if path.is_absolute():
        # Recover the repository prefix used by the unchanged source recorder.
        import re
        for index, part in enumerate(path.parts):
            if part in ('api', 'impl', 'data_layer', '_internal') or re.match(r'^l\d{4}_', part):
                root = Path(*path.parts[:index])
                break
    logical = path.relative_to(root) if root else path
    return [dict(path=logical.as_posix(), source=source)], dict(include=['**/*.py'], exclude=[]), root


def scan(suite, entry, module, args):
    if entry.endswith('should_check_file'):
        rows = suite.native('backend_coupling', 'should_check_file', [], dict(path=str(args['path'])))
        if len(rows) > 1 or any(row['code'] != 'BC_CHECK' for row in rows):
            raise AssertionError('Invalid backend path predicate')
        return bool(rows)
    files, _, _ = snapshot(args['path'], args['source'])
    rows = suite.native('backend_coupling', entry, files, dict(path=str(args['path'])))
    if any(row['code'] not in {'direct_backend_import', 'sql_in_nonsql'} for row in rows):
        raise AssertionError('Unexpected backend coupling diagnostic')
    # ast.walk visits breadth first, preserving AST field order within each depth.
    rows.sort(key=lambda row: (row['parameters']['depth'], row['parameters']['ordinal'], row['code']))
    return [module.Violation(file=str(args['path']), line=row['line'], message=row['message'],
        category=row['code']) for row in rows]


def configuration(suite):
    rows = suite.native('backend_coupling', 'configuration', [])
    tables = {}
    if not rows or any(row['code'] != 'BC_CONFIG' for row in rows):
        raise AssertionError('Native backend configuration is missing')
    for row in sorted(rows, key=lambda row: (row['parameters']['key'], row['parameters']['ordinal'])):
        values = row['parameters']
        tables.setdefault(values['key'], []).append(values['value'])
    return tables


def install(suite, module):
    """Replace framework extraction with one native batch, including native file selection."""
    import hashlib
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.linter_plugin import FileScopeSpec, LinterPlugin, PerFileFactBundle, ProjectFactView, StructuredViolation

    tables = configuration(suite)
    for key in ['BACKEND_PREFIXES', 'REGISTRATION_FILES', 'SQL_SPECIFIC_DIRS', 'SHARED_DESPITE_NAME', 'NON_SQL_BACKENDS']:
        # Native values back the original isinstance(frozenset) and set algebra assertions.
        suite.replace(module, key, frozenset(tables[key]))
    for key in ['name', 'description', 'directories']:
        def read_property(checker, key=key):
            values = configuration(suite)[key]
            return values if key == 'directories' else values[0]
        suite.replace(module.BackendCouplingChecker, key, property(read_property))
    expected = module.BACKEND_COUPLING_EXTRACTOR.extractor_id

    def plugin(root):
        def report(view):
            rows = [StructuredViolation(linter_name='backend_coupling', code=row['category'],
                path=root / row['file'], line=row['line'], message=row['message'], severity='info')
                for bundle in view.files.values() for row in bundle.facts[expected]]
            return sorted(rows, key=lambda row: (row.path.as_posix(), row.line, row.code))
        return LinterPlugin(name=tables['name'][0], enforcement='info', description=tables['description'][0],
            file_scope=FileScopeSpec(directories=(root,)),
            required_facts=(module.BACKEND_COUPLING_EXTRACTOR,), reporter=report)

    def materialize(engine, plugins, *, max_workers=None):
        if {plugin.name for plugin in plugins} != {'backend_coupling'}:
            raise AssertionError('Native backend materialization received another plugin')
        files = []
        # Read inputs only; the native BC_FILE rows decide scope and the file count.
        pending = [Path(engine.root)]
        while pending:
            directory = pending.pop()
            for path in directory.iterdir():
                if path.is_dir() and not path.is_symlink():
                    pending.append(path)
                elif path.suffix == '.py':
                    try:
                        files.append(dict(path=path.relative_to(engine.root).as_posix(), source=path.read_text(encoding='utf-8')))
                    except (OSError, UnicodeDecodeError):
                        continue
        files.sort(key=lambda row: row['path'])
        rows = suite.native('backend_coupling', 'collector', files)
        if any(row['code'] not in {'BC_FILE', 'direct_backend_import', 'sql_in_nonsql'} for row in rows):
            raise AssertionError('Unexpected native backend materialization row')
        selected = {row['path'] for row in rows if row['code'] == 'BC_FILE'}
        bundles = {}
        for file in files:
            name = file['path']
            if name not in selected:
                continue
            ordered = sorted((row for row in rows if row['path'] == name and row['code'] != 'BC_FILE'),
                key=lambda row: row['parameters']['ordinal'])
            facts = [dict(file=name, line=row['line'], message=row['message'], category=row['code']) for row in ordered]
            path = engine.root / name
            bundles[path] = PerFileFactBundle(path, hashlib.sha256(file['source'].encode()).hexdigest(), {expected: facts})
        return ProjectFactView(engine.root, bundles, (module.BACKEND_COUPLING_EXTRACTOR,))

    def extractors(root):
        # This marker must never execute: materialize consumes a native batch instead.
        def forbidden(*args):
            raise AssertionError('Native backend port must not invoke a Python collector')
        return {expected: forbidden}

    def from_fact(value):
        return module.Violation(file=str(value['file']), line=int(value['line']), message=str(value['message']), category=str(value['category']))

    suite.replace(module, 'backend_coupling_plugin', plugin)
    suite.replace(module, 'backend_coupling_fact_extractors', extractors)
    suite.replace(module, '_violation_from_fact', from_fact)
    suite.replace(FactEngine, 'materialize', materialize)
