from typing import Literal, cast
x = [[1]]
a: "sum(x, []) | sum(x, list())" = 1
b: "list[sum(x, [])]" = 1
c: Literal["sum(x, [])"] = 1
d = print("sum(x, [])")
e = cast("sum(x, [])", x)
f: "sum(x, " "[])" = 1
g = sum(x, start_=[])
