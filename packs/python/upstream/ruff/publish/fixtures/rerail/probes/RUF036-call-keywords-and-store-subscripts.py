from typing import List, Optional, TypeVar, TypedDict
from typing_extensions import TypeForm

A = TypeVar("A", covariant=None | int)
B = TypeVar("B", default=None | int)
C = TypedDict("C", x=None | int, total=None | int)
D = TypedDict("D", {"y": int}, x=None | int)
E = TypeForm(*[None | int])
F = TypedDict[{"a": None | int, "b": f(None | int)}]
d = {}
d[List[None | int]] = 1
del d[Optional[None | int]]
x = d[List[None | int]]
