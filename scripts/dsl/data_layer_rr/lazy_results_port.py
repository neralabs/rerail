"""Transport lazy-result files and collector trees; execute decisions in the native rule."""
import ast
from pathlib import Path


def ordered(rows):
    return sorted(rows, key=lambda r: (r['path'], r['parameters']['exit_order'], -r['parameters']['entry_order']))


def facts(rows):
    return [dict(file=r['path'], line=r['line'], method_name=r['parameters']['method_name'],
        class_name=r['parameters']['class_name'], return_type=r['parameters']['return_type'])
        for r in ordered(rows) if r['code'] == 'LRU_FACT']


def scan(suite, module, entry, root, paths):
    files, scopes = [], []
    for i, path in enumerate(paths):
        path = Path(path)
        actual = path if path.is_absolute() else root/path
        try:
            source = actual.read_text(encoding='utf-8')
        except (OSError, UnicodeDecodeError):
            continue
        files.append(dict(path=f'input-{i}/'+path.name, source=source))
        scopes.append(actual.relative_to(root).as_posix() if actual.is_relative_to(root) else actual.as_posix())
    options = dict(documentPaths=[f['path'] for f in files], scopePaths=scopes)
    rows = suite.native('lazy_result_usage', entry, files, options)
    return [module.Violation(file=Path(r['file']), line=r['line'], method_name=r['method_name'],
        class_name=r['class_name'], return_type=r['return_type']) for r in facts(rows)]


def graph(suite, path, tree, disabled):
    from callback_wrappers_port import ast_input
    options, _ = ast_input(tree)
    nodes = list(ast.walk(tree))
    # The columns and keys use the preorder of visit_columns, not ast.walk's breadth-first order.
    pending, nodes = [tree], []
    while pending:
        n = pending.pop()
        nodes.append(n)
        pending.extend(reversed(list(ast.iter_child_nodes(n))))
    annotations = [(i, ast.unparse(n)) for i, n in enumerate(nodes) if options['astFields'][i] == 'returns']
    transport = 'def _transport('+', '.join(f'e{i}: {s}' for i, s in annotations)+'):\n    pass\n' if annotations else 'pass\n'
    options.update(originalPath=str(path), disabled=str(disabled).lower())
    rows = suite.native('lazy_result_usage', 'collector', [dict(path='_transport.py', source=transport)], options)
    return facts(rows)


def collector_result(module, values, suite=None):
    """Deliver a supplied tree through the original collector callback protocol."""
    from scripts.lint_lib.ast_collectors import AstTraversalContext
    tree, path, source = values['tree'], values['filepath'], values.get('source', '')
    disabled = values.get('disabled', False)
    if suite:
        return graph(suite, path, tree, disabled)
    collector = module.LazyReturnCollector(path, disabled=disabled)
    parents, children, pending = {tree: None}, {}, [tree]
    while pending:
        n = pending.pop()
        children[n] = tuple(ast.iter_child_nodes(n))
        for c in children[n]:
            parents[c] = n
        pending.extend(reversed(children[n]))
    context = AstTraversalContext(path, source, parents, children)
    collector.start_file(path, source, context)
    pending = [(tree, False)]
    while pending:
        n, leaving = pending.pop()
        if leaving:
            collector.on_exit_node(n, context)
            handler = getattr(collector, 'on_exit_'+type(n).__name__, None)
            if handler:
                handler(n, context)
        else:
            collector.on_node(n, context)
            handler = getattr(collector, 'on_'+type(n).__name__, None)
            if handler:
                handler(n, context)
            pending.append((n, True))
            pending.extend((c, False) for c in reversed(children[n]))
    return collector.finish_file()


def install(suite, module):
    def find_violations(root):
        root = Path(root)
        return scan(suite, module, 'find_violations', root, list(root.rglob('*.py')))
    def for_files(root, files):
        return scan(suite, module, '_violations_for_files', Path(root), files)
    def scan_source(path, _source, root, tree=None):
        return for_files(root, (path,))
    def should_skip(filepath):
        rows = suite.native('lazy_result_usage', 'should_skip', [], dict(pathProbe=str(filepath)))
        return bool(rows)
    for name, value in [('find_violations', find_violations), ('_violations_for_files', for_files),
            ('_scan_source', scan_source), ('should_skip', should_skip)]:
        suite.replace(module, name, value)
