"""Serialize caller-supplied AST values into flat columns; perform no linter decisions."""
import ast
import json


def columns(tree):
    result = {name: [] for name in ['astKinds', 'astIds', 'astAttrs', 'astNames', 'astLines',
                                   'astParents', 'astFields', 'astOrdinals', 'astValueTypes']}
    pending = [(-1, '', 0, tree)]
    missing = object()
    while pending:
        parent, field, ordinal, node = pending.pop()
        index = len(result['astKinds'])
        values = [type(node).__name__, getattr(node, 'id', ''), getattr(node, 'attr', ''),
                  getattr(node, 'name', ''), str(getattr(node, 'lineno', 0)), str(parent), field,
                  str(ordinal), type(getattr(node, 'value', missing)).__name__]
        for name, value in zip(result, values):
            result[name].append(value)
        children = []
        for name, value in ast.iter_fields(node):
            if isinstance(value, ast.AST):
                children.append((index, name, 0, value))
            elif isinstance(value, list):
                children.extend((index, name, i, child) for i, child in enumerate(value) if isinstance(child, ast.AST))
        # Store occurrences, matching iter_child_nodes; shared AST expressions retain visit multiplicity.
        pending.extend(reversed(children))
    return result


def syntax_columns(tree):
    """Encode supplied syntax, locations and literal fields; make no detector decisions.

    Receiver ASTs are encoded as annotation syntax for the native canonical printer.
    This is external test-argument transport. The .rr rule never invokes Python.
    """
    result = columns(tree)
    extra = {name: [] for name in ['astArgNames', 'astStringJson', 'astColumns']}
    receivers = []
    pending = [tree]
    while pending:
        node = pending.pop()
        index = len(extra['astArgNames'])
        extra['astArgNames'].append(getattr(node, 'arg', ''))
        value = getattr(node, 'value', None)
        extra['astStringJson'].append(json.dumps(value, ensure_ascii=True) if isinstance(value, str) else '')
        extra['astColumns'].append(str(getattr(node, 'col_offset', 0)))
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            receivers.append((index, ast.unparse(node.func.value)))
        pending.extend(reversed(list(ast.iter_child_nodes(node))))
    result.update(extra)
    # The native printer gets complete expression inputs, rather than expected diagnostic fragments.
    result['receiverTransport'] = ('def _receiver_transport(' + ', '.join(
        f'call{index}: {source}' for index, source in receivers) + '):\n    pass\n') if receivers else 'pass\n'
    return result
