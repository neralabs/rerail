from __future__ import division
from typing import Annotated, Literal, TypedDict

x1: Annotated[int, Literal["a" | b], b"x" | c]
x2: Annotated[Literal["a" | b], 1] | "c"

TD1 = TypedDict("TD1", a="A" | B)
TD2 = TypedDict("TD2", None, total="A" | B, b=Literal["A" | B])
TD3 = TypedDict("TD3", {"x": int}, a="A" | B)
TD4 = TypedDict(name="TD4", a="A" | B)
