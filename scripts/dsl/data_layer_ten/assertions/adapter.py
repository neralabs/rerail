# SPDX-License-Identifier: Apache-2.0
"""Run original unit assertions with SQL decisions and AST objects as DTOs."""
import ast
import hashlib
import inspect
import json
import textwrap
from functools import wraps


GRAPH = ('_non_docstring_stmts', '_decorator_names', '_find_class', '_find_last_method',
         '_function_mentions_attr', '_function_mentions_name', '_function_mentions_any_name', '_class_has_assignment')
BOOLS = {'_function_mentions_attr', '_function_mentions_name', '_function_mentions_any_name',
         '_class_has_assignment', '_is_data_path', '_tree_uses_matching_contracts'}


def preorder(root):
    stack = [root]
    while stack:
        node = stack.pop()
        yield node
        stack.extend(reversed(list(ast.iter_child_nodes(node))))


def exception_value(error):
    return {'exception': {'type': type(error).__name__, 'message': str(error),
                         **{name: getattr(error, name) for name in ('msg', 'lineno', 'offset', 'text', 'end_lineno', 'end_offset')}}}


def inputs(backend, check, entry, module, arguments):
    if entry == '_parse_source':
        return [{'path': 'helper.py', 'source': arguments['source']}], {}, {}
    if entry == '_is_data_path':
        return [{'path': 'helper.py', 'source': ''}], {'path': arguments['path'].relative_to(module.ROOT).as_posix()}, {}
    if entry == '_approval_entry_errors':
        return [{'path': 'helper.py', 'source': ''}], {'entries': arguments['entries']}, {}
    owner = next(value for value in arguments.values() if isinstance(value, ast.AST))
    if owner not in backend.node_ids or owner not in backend.trees:
        raise AssertionError('The unit assertion supplied an AST without captured source provenance')
    ordinal = backend.node_ids[owner]
    objects = {ordinal + offset: node for offset, node in enumerate(preorder(owner))}
    parameters = {'owner': ordinal}
    for key in ('symbol_name', 'attr_name', 'class_name', 'method_name'):
        if key in arguments:
            parameters['symbol'] = arguments[key]
    if 'symbol_names' in arguments:
        parameters['symbols'] = list(arguments['symbol_names'])
    return [{'path': 'helper.py', 'source': backend.trees[owner]}], parameters, objects


def contract(entry):
    return 'batch-ten-helper-' + {'_parse_source': 'parse', '_is_data_path': 'path',
        '_approval_entry_errors': 'approvals', '_tree_uses_matching_contracts': 'matching'}.get(entry, 'ast')


def encode(entry, result, objects):
    if entry == '_parse_source':
        return {'module': isinstance(result, ast.Module)}
    if entry in BOOLS:
        return result
    if entry == '_decorator_names':
        return sorted(result)
    if entry == '_approval_entry_errors':
        return result
    indices = {id(node): ordinal for ordinal, node in objects.items()}
    if entry == '_non_docstring_stmts':
        return [indices[id(node)] for node in result]
    return None if result is None else indices[id(result)]


def decode(entry, rows, files, objects):
    values = [finding['variables']['row'] for finding in rows]
    if entry == '_parse_source':
        if len(values) != 1:
            raise AssertionError('Native syntax contract must publish its parse result')
        error = json.loads(values[0]['helper_value'])
        if error is not None:
            raise SyntaxError(error['msg'], ('<unknown>', error['lineno'], error['offset'], error['text'], error['end_lineno'], error['end_offset']))
        # This constructs the Python DTO required by the unchanged assertions.
        # Parse success or failure was already decided by the native contract.
        try:
            return ast.parse(files[0]['source'])
        except SyntaxError as error:
            raise AssertionError('Native parser accepted invalid source during DTO construction') from error
    if entry in BOOLS:
        if len(values) != 1 or values[0]['helper_value'] not in (0, 1):
            raise AssertionError('Native Boolean contract must publish exactly one Boolean')
        return bool(values[0]['helper_value'])
    if entry == '_decorator_names':
        return {value['helper_value'] for value in values}
    if entry == '_approval_entry_errors':
        return [value['helper_value'] for value in sorted(values, key=lambda value: value['identity'])]
    values.sort(key=lambda value: value['node_id'])
    if any(value['node_id'] not in objects for value in values):
        raise AssertionError('Native AST result does not refer to the supplied Python DTO subtree')
    if entry == '_non_docstring_stmts':
        return [objects[value['node_id']] for value in values]
    if len(values) > 1:
        raise AssertionError('Native node-selection contract returned more than one node')
    return objects[values[0]['node_id']] if values else None


def wrapper(backend, check, entry, module, original):
    signature = inspect.signature(original)

    @wraps(original)
    def run(*args, **kwargs):
        # Original detector recording must keep the actual collector behavior.
        # Its nested utility calls are not independent upstream unit assertions.
        if backend.oracle and (getattr(backend, 'adapter_depth', 0) or getattr(backend, 'helper_depth', 0)):
            return original(*args, **kwargs)
        arguments = dict(signature.bind(*args, **kwargs).arguments)
        files, parameters, objects = inputs(backend, check, entry, module, arguments)
        error = None
        try:
            if backend.oracle:
                backend.helper_depth = getattr(backend, 'helper_depth', 0) + 1
                try:
                    result = original(*args, **kwargs)
                finally:
                    backend.helper_depth -= 1
            else:
                with backend.protocol_lock:
                    from native_suite import NativeSuite
                    offset = len(backend.calls)
                    try:
                        rows = NativeSuite.execute(backend, check, entry, files, contract=contract(entry), parameters=parameters)
                    finally:
                        backend.helper_calls.extend(backend.calls[offset:])
                        del backend.calls[offset:]
                result = decode(entry, rows, files, objects)
        except SyntaxError as exception:
            error = exception
        expected = exception_value(error) if error is not None else encode(entry, result, objects)
        record = dict(test=backend.item.nodeid, check=check, entry=entry, files=files, parameters=parameters, expected=expected)
        backend.helper_records.append(record)
        if not backend.oracle:
            matches = [r for r in backend.helper_baseline if (r['test'], r['check'], r['entry']) == (record['test'], check, entry)]
            ordinal = sum((r['test'], r['check'], r['entry']) == (record['test'], check, entry) for r in backend.helper_records[:-1])
            assert ordinal < len(matches), ('Unrecorded helper execution', record)
            assert record == matches[ordinal], ('Complete helper parity', record, matches[ordinal])
        if error is not None:
            raise error
        return result

    if hasattr(original, 'cache_clear'):
        run.cache_clear = original.cache_clear
    return run


def install(backend):
    backend.helper_records, backend.helper_calls, backend.assertion_translations = [], [], []
    backend.helper_code_names = {}
    if 'graph_lazy_sql_plan' in backend.modules:
        backend.helper_code_names[backend.modules['graph_lazy_sql_plan']._parse_source.__code__] = 'graph_lazy_sql_plan:_parse_source'
    for check, entries in [('graph_lazy_sql_plan', GRAPH), ('schema_introspection_data_paths', ('_is_data_path', '_approval_entry_errors')),
                           ('matching_architecture', ('_tree_uses_matching_contracts',))]:
        module = backend.modules.get(check)
        if module is None:
            continue
        for entry in entries:
            original = getattr(module, entry)
            backend.helper_code_names[inspect.unwrap(original).__code__] = check + ':' + entry
            backend.replace(module, entry, wrapper(backend, check, entry, module, original))


def translate(backend, items):
    for item in items:
        name = getattr(item, 'originalname', None) or item.name
        if 'test_lint_graph_lazy_sql_plan' in item.nodeid:
            module = item.module
            if not getattr(module._parse_source, '_native_helper_wrapper', False):
                replacement = wrapper(backend, 'graph_lazy_sql_plan', '_parse_source', backend.modules['graph_lazy_sql_plan'], module._parse_source)
                replacement._native_helper_wrapper = True
                backend.replace(module, '_parse_source', replacement)
        if name != 'test_matching_contract_detection_traverses_each_module_once' or backend.oracle:
            continue
        original = textwrap.dedent(inspect.getsource(item.obj))
        tree = ast.parse(original)
        function = next(node for node in tree.body if isinstance(node, ast.FunctionDef))
        function.decorator_list = []
        replaced = 0

        class Counter(ast.NodeTransformer):
            def visit_Assert(self, node):
                nonlocal replaced
                if ast.unparse(node.test) == 'walk_nodes.call_count == 1':
                    node.test = ast.parse("_native_metric('pythonAstModuleWalks') == 1 and _native_metric('pythonAstParses') == 1", mode='eval').body
                    replaced += 1
                return node

        tree = ast.fix_missing_locations(Counter().visit(tree))
        assert replaced == 1, 'The original instrumentation assertion changed'
        namespace = dict(item.obj.__globals__)

        def metric(key):
            metrics = backend.last_metrics.get('workspaceProfile', backend.last_metrics)
            if key not in metrics:
                raise AssertionError('Native helper execution did not publish ' + key)
            return metrics[key]

        namespace['_native_metric'] = metric
        exec(compile(tree, inspect.getsourcefile(item.obj), 'exec'), namespace)
        item.obj = namespace[function.name]
        digest = lambda text: hashlib.sha256(text.encode()).hexdigest()
        backend.assertion_translations.append(dict(test=item.nodeid, originalHash=digest(original), translatedHash=digest(ast.unparse(tree)),
                                                   nativeCounters=['pythonAstModuleWalks', 'pythonAstParses']))
