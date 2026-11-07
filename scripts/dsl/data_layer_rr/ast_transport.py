"""Serialize caller-supplied AST values into flat columns; perform no linter decisions."""
import ast


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
