# SPDX-License-Identifier: Apache-2.0
"""Read pinned literal contracts, including unpacked set literals, without imports."""
import ast
from data_layer_eight.contracts import constants as ordinary_constants


def constants(path):
    values=ordinary_constants(path)
    def members(node):
        if isinstance(node,ast.Name):return values[node.id]
        if isinstance(node,(ast.Set,ast.Tuple,ast.List)):
            return [value for item in node.elts for value in (members(item.value) if isinstance(item,ast.Starred) else [ast.literal_eval(item)])]
        raise ValueError('Expected a literal collection')
    for node in ast.parse(path.read_text()).body:
        if not isinstance(node,(ast.Assign,ast.AnnAssign)):continue
        value=node.value
        if not isinstance(value,ast.Call) or not isinstance(value.func,ast.Name) or value.func.id not in {'frozenset','set','tuple','list'} or len(value.args)!=1 or value.keywords:continue
        try:result={'frozenset':frozenset,'set':set,'tuple':tuple,'list':list}[value.func.id](members(value.args[0]))
        except (KeyError,ValueError,TypeError):continue
        for target in node.targets if isinstance(node,ast.Assign) else [node.target]:
            if isinstance(target,ast.Name):values[target.id]=result
    return values
