# SPDX-License-Identifier: Apache-2.0
"""Read literal source contracts without importing or executing the source module."""
import ast


def constants(path):
    values={}
    def read(node):
        if isinstance(node,ast.Constant):return node.value
        if isinstance(node,ast.Name):return values[node.id]
        if isinstance(node,(ast.Tuple,ast.List,ast.Set)):
            constructor={ast.Tuple:tuple,ast.List:list,ast.Set:set}[type(node)]
            return constructor(read(item) for item in node.elts)
        if isinstance(node,ast.Dict):return {read(k):read(v) for k,v in zip(node.keys,node.values)}
        if isinstance(node,ast.BinOp):
            left,right=read(node.left),read(node.right)
            if isinstance(node.op,ast.Add):return left+right
            if isinstance(node.op,ast.BitOr):return left|right
        if isinstance(node,ast.Call) and isinstance(node.func,ast.Name) and node.func.id in {'frozenset','set','tuple','list'} and len(node.args)<=1 and not node.keywords:
            return {'frozenset':frozenset,'set':set,'tuple':tuple,'list':list}[node.func.id](read(node.args[0]) if node.args else ())
        raise ValueError('Contract is not a literal')
    for node in ast.parse(path.read_text()).body:
        if isinstance(node,(ast.Assign,ast.AnnAssign)):
            targets=node.targets if isinstance(node,ast.Assign) else [node.target]
            try:value=read(node.value)
            except (KeyError,ValueError,TypeError,AttributeError):continue
            for target in targets:
                if isinstance(target,ast.Name):values[target.id]=value
    return values
