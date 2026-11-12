"""Recognize passive builtin-only type alias thunks without evaluating their values."""
import ast
import builtins
from functools import lru_cache
from pathlib import Path
from typing import TypeAliasType

BUILTINS=frozenset({'dict','list','tuple','set','frozenset','str','bytes','int','float','bool','object','type'})


@lru_cache(maxsize=128)
def alias_names(filename,line,name):
    try:tree=ast.parse(Path(filename).read_text())
    except (OSError,UnicodeError,SyntaxError):return None
    aliases=[n for n in tree.body if isinstance(n,ast.TypeAlias) and n.lineno==line and n.name.id==name]
    if len(aliases)!=1 or aliases[0].type_params:return None
    nodes=list(ast.walk(aliases[0].value))
    allowed=(ast.Name,ast.Subscript,ast.Tuple,ast.BinOp,ast.BitOr,ast.Load,ast.Constant)
    if any(not isinstance(n,allowed) for n in nodes):return None
    if any(isinstance(n,ast.Constant) and n.value is not None and n.value is not Ellipsis for n in nodes):return None
    names=frozenset(n.id for n in nodes if isinstance(n,ast.Name))
    return names if names<=BUILTINS else None


def passive_type_alias(frame):
    code=frame.f_code;namespace=frame.f_globals
    alias=namespace.get(code.co_name)
    if not isinstance(alias,TypeAliasType) or alias.__module__!=namespace.get('__name__'):return False
    if code.co_name!=code.co_qualname or alias.__name__!=code.co_name:return False
    names=alias_names(code.co_filename,code.co_firstlineno,code.co_name)
    if names is None:return False
    return all(namespace.get(name,frame.f_builtins.get(name)) is getattr(builtins,name) for name in names)
