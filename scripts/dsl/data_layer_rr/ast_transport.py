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


def scalar_columns(tree):
    """Flat AST fields shared by the transaction visitor contracts."""
    data=columns(tree);nodes=[];pending=[tree]
    while pending:
        node=pending.pop();nodes.append(node);pending.extend(reversed(list(ast.iter_child_nodes(node))))
    for key,attr in [('astColumns','col_offset'),('astArgNames','arg'),('astKeywords','arg'),('astAsNames','asname')]:
        data[key]=[str(getattr(n,attr,0)) if key=='astColumns' else str(getattr(n,attr,'') or '') for n in nodes]
    data.update(astListNodes=[],astListNames=[],astListOrdinals=[])
    for index,node in enumerate(nodes):
        for ordinal,name in enumerate(getattr(node,'names',[])):
            if isinstance(name,str):
                data['astListNodes'].append(str(index));data['astListNames'].append(name);data['astListOrdinals'].append(str(ordinal))
    return data


def visit_columns(tree):
    """Per-occurrence completion order and ASCII JSON text of literal values; no detector decisions."""
    data=scalar_columns(tree);pending=[(tree,-1,False)];nodes=[];exits={}
    while pending:
        node,index,leaving=pending.pop()
        if leaving:exits[index]=len(exits);continue
        index=len(nodes);nodes.append(node);pending.append((node,index,True))
        pending.extend((c,-1,False) for c in reversed(list(ast.iter_child_nodes(node))))
    data['astExitOrder']=[str(exits[i]) for i in range(len(nodes))]
    data['astLiteralJson']=[json.dumps(str(n.value),ensure_ascii=True) if isinstance(n,ast.Constant) else '' for n in nodes]
    return data
