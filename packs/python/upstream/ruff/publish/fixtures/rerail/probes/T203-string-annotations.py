from typing import List, Tuple, TypeVar, cast
from pprint import pprint
import sys

x: "pprint(1)" = 1
y: "Tuple[pprint(2), pprint(3)]" = 1
z: List["pprint(4)"] = []


def f(a: "pprint(5, stream=sys.stderr)", b: "pprint(6, stream=open('x'))") -> "pprint(7)":
    pass


def g(pprint):
    q: "pprint(8)" = 1


T = TypeVar("T", bound="pprint(9)")
w = cast("pprint(10)", 1)
u = "pprint(11)"
