from typing import TypeVar, ParamSpec, TypeVarTuple
def f():
    _A = TypeVar("_A")
    x = _A
    _A = TypeVar("_A")
    return locals()
def g():
    _B = TypeVar("_B")
    def h():
        return locals()
    return h
def k():
    _C = TypeVar("_C")
    locals
