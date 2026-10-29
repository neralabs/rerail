from typing import List, cast, TypeVar
def f(a: "foo + [1]") -> "[1] + foo": ...
x: "foo + [1]" = foo + [2]
y: List["foo + [1]"] = []
z = cast("(1,) + foo", w)
T = TypeVar("T", bound="[1] + foo")
v: "[1] + foo | bar + (2,)" = 1
u: "Literal['[1] + foo']" = 1
s: "foo + 1" = 1
t: "1 + [1] + foo" = 1
r: "  [1] + foo" = 1
q: "[1]" "+ foo" = 1
p: "[1] + \x66oo" = 1
