"""Transport FieldInfo file inputs, external AST DTOs, and result dictionaries."""
import ast
import inspect
import types
from pathlib import Path

CHECK = 'unresolved_forward_refs'


def analyzed_state(owner, returned):
    """Expose the complete observable instance state without invoking source methods."""
    return dict(returned=returned, file_path=owner.file_path, findings=owner.findings, source_lines=owner.source_lines,
                current_function=owner.current_function, current_function_lineno=owner.current_function_lineno,
                resolved_hint_vars=sorted(owner.resolved_hint_vars))


def supplied_snapshot(owner, tree):
    from ast_transport import columns
    root = Path(owner.file_path).parent
    try:
        source = owner.file_path.read_text()
    except (OSError, UnicodeError):
        files = []
    else:
        files = [dict(path=Path(owner.file_path).name, source=source)]
    return files, dict(include=['**/*.py'], exclude=[]), root, columns(tree)


def identity_graph(container, target):
    """Serialize object identities and child edges; native reachability answers containment."""
    objects, pending, seen, parents, children = {}, [container, target], set(), [], []
    def key(node):
        return str(objects.setdefault(id(node), len(objects)))
    container_id, target_id = key(container), key(target)
    while pending:
        node = pending.pop()
        if id(node) in seen:
            continue
        seen.add(id(node))
        for child in ast.iter_child_nodes(node):
            parents.append(key(node))
            children.append(key(child))
            pending.append(child)
    return dict(containerId=container_id, targetId=target_id, graphParents=parents, graphChildren=children)


def scan(suite, entry, module, arguments):
    if entry == 'AnnotationUsageFinder.analyze':
        from ast_transport import columns
        owner = arguments['self']
        options = columns(arguments['tree'])
        read_error = None
        try:
            source = owner.file_path.read_text()
        except UnicodeDecodeError:
            source = ''
        except OSError as error:
            read_error, source = error, ''
        options.update(providedSource=source, sourceLines=source.splitlines(),
                       readError=str(read_error is not None).lower())
        rows = suite.native(CHECK, entry, [], options)
        if any(row['code'] == 'UF_READ_ERROR' for row in rows):
            if len(rows) != 1 or read_error is None:
                raise AssertionError('Invalid supplied-AST read failure')
            raise read_error
        if any(row['code'] != 'unresolved_forward_ref' for row in rows):
            raise AssertionError('Invalid native supplied-AST finding')
        owner.findings.extend(dict(file=str(owner.file_path), line=row['line'], function=row['parameters']['function'],
            problem=row['parameters']['problem'], source=row['parameters']['source'])
            for row in sorted(rows, key=lambda row: row['parameters']['ordinal']))
        return None
    if entry.startswith('AnnotationUsageFinder.'):
        options, files = {}, []
        if entry.endswith('._node_contains'):
            options = identity_graph(arguments['container'], arguments['target'])
        elif entry.endswith(('._is_using_resolved_hints', '._is_assignment_from_resolved')):
            pass  # Constant source predicates have no relevant input fields.
        else:
            supplied = arguments.get('parent', arguments.get('node'))
            # Tests supply Python AST values. Printing is test-input transport, never a rule callback.
            files = [dict(path='helper.py', source='_rr_input = ' + ast.unparse(supplied) + '\n')]
        rows = suite.native(CHECK, entry, files, options)
        if len(rows) > 1 or any(row['code'] != 'UF_HELPER_TRUE' for row in rows):
            raise AssertionError('Invalid native unresolved-reference helper result')
        return bool(rows)
    path = Path(arguments['file_path'])
    read_error = None
    try:
        source = path.read_text(encoding='utf-8')
    except UnicodeDecodeError:
        source = ''
    except OSError as error:
        read_error, source = error, ''
    rows = suite.native(CHECK, entry, [dict(path='input.py', source=source)],
                        dict(suffix=path.suffix, isFile=str(path.is_file()).lower(),
                             relativePath=str(not path.is_absolute()).lower(),
                             readError=str(read_error is not None).lower()))
    if any(row['code'] == 'UF_READ_ERROR' for row in rows):
        if len(rows) != 1 or read_error is None:
            raise AssertionError('Invalid native unresolved-reference read failure')
        raise read_error
    if any(row['code'] != 'unresolved_forward_ref' for row in rows):
        raise AssertionError('Unexpected native unresolved-reference finding')
    return [dict(file=str(path), line=row['line'], function=row['parameters']['function'],
                 problem=row['parameters']['problem'], source=row['parameters']['source'])
            for row in sorted(rows, key=lambda row: row['parameters']['ordinal'])]


def install(suite, module):
    rows = suite.native(CHECK, 'configuration', [])
    tables = {}
    for row in rows:
        if row['code'] != 'UF_CONFIG':
            raise AssertionError('Invalid native unresolved-reference configuration')
        tables.setdefault(row['parameters']['key'], set()).add(row['parameters']['value'])
    for key in ['TYPE_INSPECTION_FUNCTIONS', 'RESOLVED_PATTERNS']:
        suite.replace(module, key, tables[key])
    if not hasattr(suite, 'configuration_bindings'):
        suite.configuration_bindings = {}
    suite.configuration_bindings[CHECK] = dict(nativeCall=len(suite.calls)-1,
        names=['TYPE_INSPECTION_FUNCTIONS', 'RESOLVED_PATTERNS'])
    transfer_cli(suite, module)


def transfer_cli(suite, module):
    """Retain source argument parsing and rendering; replace discovery and detection with batches."""
    def findings(root, scan_paths):
        candidates, modes, relative = [], [], []
        for path in scan_paths:
            if not path.exists():
                continue
            paths = [path] if path.is_file() else sorted(path.rglob('*'))
            for child in paths:
                candidates.append(child)
                modes.append('direct' if path.is_file() else 'recursive')
                relative.append(child.relative_to(path).as_posix() if not path.is_file() else child.name)
        selected = suite.native(CHECK, 'cli_selection', [], dict(cliNames=[p.name for p in candidates],
            cliModes=modes, cliRelativePaths=relative))
        if any(row['code'] != 'UF_CLI_FILE' for row in selected):
            raise AssertionError('Invalid native unresolved-reference CLI scope')
        paths = [candidates[row['parameters']['ordinal']] for row in sorted(selected, key=lambda r: r['parameters']['ordinal'])]
        files, names = [], {}
        for i, path in enumerate(paths):
            # The source calls analyze_file here. Preserve its I/O error and Unicode contracts.
            try:
                source = path.read_text(encoding='utf-8')
            except UnicodeDecodeError:
                source = ''
            except OSError:
                scan(suite, 'analyze_file', module, dict(file_path=path))
                raise AssertionError('Native I/O failure unexpectedly returned')
            buffer = f'input-{i}.py'
            names[buffer] = path
            files.append(dict(path=buffer, source=source))
        rows = suite.native(CHECK, 'cli_scan', files, dict(logicalFiles=list(names),
            fileSuffixes=[p.suffix for p in paths], relativeFiles=[key for key,p in names.items() if not p.is_absolute()]))
        grouped = {key: [] for key in names}
        for row in rows:
            if row['code'] != 'unresolved_forward_ref':
                raise AssertionError('Unexpected unresolved-reference CLI finding')
            grouped[row['path']].append(row)
        result = []
        for key, path in names.items():
            values = [dict(file=str(path), line=row['line'], function=row['parameters']['function'],
                problem=row['parameters']['problem'], source=row['parameters']['source'])
                for row in sorted(grouped[key], key=lambda row: row['parameters']['ordinal'])]
            suite.compare_original(CHECK, 'analyze_file', module, (path,), {}, values)
            result.extend(values)
        return result

    original = inspect.getsource(module.cli_main)
    tree = ast.parse(original)
    body = tree.body[0].body
    loops = [i for i, node in enumerate(body) if isinstance(node, ast.For)
             and isinstance(node.target, ast.Name) and node.target.id == 'scan_path']
    if len(loops) != 1 or ast.unparse(body[loops[0]].iter) != 'scan_paths':
        raise AssertionError('Review the changed source CLI discovery loop')
    replacement = ast.parse('all_findings = _rr_cli_findings(root_dir, scan_paths)').body[0]
    body[loops[0]] = ast.copy_location(replacement, body[loops[0]])
    ast.fix_missing_locations(tree)
    compiled = compile(tree, module.__file__, 'exec')
    code = next(value for value in compiled.co_consts if isinstance(value, types.CodeType) and value.co_name == 'cli_main')
    # Keep the live module globals: upstream tests replace discovery inputs with monkeypatch.
    module.__dict__['_rr_cli_findings'] = findings
    cli = types.FunctionType(code, module.__dict__)
    cli.__annotations__ = dict(module.cli_main.__annotations__)
    suite.replace(module, 'cli_main', cli)
    if not hasattr(suite, 'host_transfers'):
        suite.host_transfers = []
    from migration import sha
    suite.host_transfers.append(dict(check=CHECK, entry='cli_main', sourceSha256=sha(original.encode()),
        translatedSha256=sha(ast.unparse(tree).encode()), operation='Replace discovery/detection loop with two native batches; preserve parsing and rendering'))
