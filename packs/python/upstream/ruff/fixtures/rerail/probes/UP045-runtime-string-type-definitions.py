from typing import Optional, List

a = List["Optional[int]"]
b = Optional["Optional[int]"]
c = list["Optional[int]"]
d = dict[str, "Optional[int]"]
e = foo["Optional[int]"]
type X = "Optional[int]"
type Y = Optional["A"]
type Z = list[Optional["A"]]
def f[T: Optional["A"]](x: T) -> T: ...
def g[T: "Optional[int]"](x: T) -> T: ...
class C[T: Optional["A"] = Optional["B"]]: ...
