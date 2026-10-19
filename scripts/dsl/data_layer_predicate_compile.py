# SPDX-License-Identifier: Apache-2.0
"""Lower pinned path/name allowlist predicates to authored AST conditions.

This authoring helper accepts constant assignments, boolean tests and loops over
constant path/name pairs. Unsupported source syntax raises an error.
"""
from __future__ import annotations
import ast
from pathlib import Path
import re


def compile_allowlist(module, owner, method):
    tree = ast.parse(Path(module.__file__).read_text())
    cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == owner)
    function = next(n for n in cls.body if isinstance(n, ast.FunctionDef) and n.name == method)
    env = {'name':('field','name'), 'path':('field','path')}
    positives = []
    def combine(op, values):
        values = [v for v in values if v is not (op == 'all')]
        if any(v is (op != 'all') for v in values):return op != 'all'
        return {op:values} if values else op == 'all'
    def negate(value):return not value if isinstance(value, bool) else {'not':value}
    def field_match(field, pattern):
        return {'filePath':{'regex':pattern}} if field == 'path' else {'has':{'field':'name','regex':pattern}}
    def value(node, values):
        if isinstance(node, ast.Name):return values[node.id]
        if isinstance(node, ast.Attribute) and ast.unparse(node) == 'self.file_path':return ('field','path')
        if isinstance(node, ast.BoolOp):return combine('all' if isinstance(node.op, ast.And) else 'any', [value(n, values) for n in node.values])
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not):return negate(value(node.operand, values))
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            target = value(node.func.value, values)
            if not isinstance(target, tuple) or target[0] != 'field':raise ValueError(ast.unparse(node))
            literal = value(node.args[0], values)
            terms = literal if isinstance(literal, (tuple,list,set)) else [literal]
            if node.func.attr not in ('startswith','endswith'):raise ValueError(ast.unparse(node))
            pattern = '(?:'+'|'.join(re.escape(s) for s in terms)+')'
            return field_match(target[1], ('^'+pattern) if node.func.attr == 'startswith' else (pattern+'$'))
        if isinstance(node, ast.Compare) and len(node.ops) == 1:
            left, right = value(node.left, values), value(node.comparators[0], values)
            op = node.ops[0]
            if isinstance(op, (ast.In,ast.NotIn)):
                if isinstance(right, tuple) and right[0] == 'field':result = field_match(right[1], re.escape(left))
                elif isinstance(left, tuple) and left[0] == 'field':result = field_match(left[1], '^(?:'+'|'.join(re.escape(s) for s in right)+')$')
                else:result = left in right
                return negate(result) if isinstance(op, ast.NotIn) else result
            if isinstance(op, (ast.Eq,ast.NotEq)):
                if isinstance(left, tuple) and left[0] == 'field':result = field_match(left[1], '^'+re.escape(right)+'$')
                else:result = left == right
                return negate(result) if isinstance(op, ast.NotEq) else result
        try:return ast.literal_eval(node)
        except (ValueError,TypeError):raise ValueError('Unsupported pinned predicate expression: '+ast.unparse(node)) from None
    def visit(statements, values, condition):
        for node in statements:
            if isinstance(node, ast.Expr) and isinstance(node.value, ast.Constant):continue
            if isinstance(node, ast.Assign):
                for target in node.targets:values[target.id] = value(node.value, values)
            elif isinstance(node, ast.If):
                test = value(node.test, values)
                visit(node.body, values.copy(), combine('all',[condition,test]))
                visit(node.orelse, values.copy(), combine('all',[condition,negate(test)]))
            elif isinstance(node, ast.For):
                names = [n.id for n in node.target.elts]
                for row in value(node.iter, values):visit(node.body, {**values,**dict(zip(names,row,strict=True))}, condition)
            elif isinstance(node, ast.Return):
                if value(node.value, values) is True:positives.append(condition)
                elif not (isinstance(node.value,ast.Constant) and node.value.value is False):raise ValueError(ast.unparse(node))
            else:raise ValueError('Unsupported pinned predicate statement: '+ast.unparse(node))
    visit(function.body, env, True)
    result = combine('any',positives)
    return {'regex':r'(?s).*' if result else r'\b\B'} if isinstance(result,bool) else result
