from multiprocessing import Value
from typing import Annotated, List, Literal, TypeVar, cast


def g(*a):
    return a


a: "g(True, False)"
b: "Annotated[int, g(\x54rue)]"
c: "Annotated[int, " "g(True)]"
d: "List['g(True)']"
e: "Literal['g(True)']"
f = cast("Annotated[int, g(True)]", 1)
T = TypeVar("T", bound="Annotated[int, g(False)]")
h: "Annotated[int, obj.get(True), obj.set_x(False), g((True))]"
i: """Annotated[int,
    g(True)]"""
j = "Annotated[int, g(True)]"
k: List["Annotated[int, g(True)]"]
v: "Annotated[int, Value('b', True, False)]"
