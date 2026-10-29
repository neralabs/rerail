import typing as t
import typing_extensions as te
from typing import TypeVar as TV, ParamSpec

U = t.TypeVar("U", bound=int)
P = te.ParamSpec("P")
V = TV("V")
Q = ParamSpec("Q")
_S = t.TypeVar("_S")
W: t.TypeVar = t.TypeVar("W")
A = B = t.TypeVar("A")
E, F = t.TypeVar("E")
(C) = t.TypeVar("C")
D = (t.TypeVar("D"))
