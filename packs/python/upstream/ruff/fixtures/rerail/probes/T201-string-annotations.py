from typing import List, Tuple, TypeVar, cast
import sys

x: "print(1)" = 1
y: "Tuple[print(2), print(3)]" = 1
z: List["print(4)"] = []


def f(a: "print(5, file=sys.stderr)", b: "print(6, file=open('x'))") -> "print(7)":
    pass


T = TypeVar("T", bound="print(8)")
w = cast("print(9)", 1)
v: "List[Literal['print(10)']]" = []
u = "print(11)"
s: "print" = 1
