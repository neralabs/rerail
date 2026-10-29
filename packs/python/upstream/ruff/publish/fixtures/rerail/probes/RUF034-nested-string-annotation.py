from typing import Literal, Annotated

x: "list['a if b else a']" = []
y: Literal["a if b else a"] = 1
z: Annotated[int, "a if b else a"] = 1
print("a if b else a")
w = (a and b and c) if t else ((a and b) and c)
