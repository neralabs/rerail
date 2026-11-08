from typing import Generic, TypeVar

_T = TypeVar("_T")

class Box(Generic[_T]):
    def __init__(self, v: _T) -> None: ...

class Var(Generic[_T]):
    def __new__(cls, name: str) -> Var[_T]: ...
