import sys
from typing import Literal


def f(x: Literal["a" "b", "c"], y: list["a" "b"] = ()) -> tuple["a" "b", int]: ...


v: Literal["a" "b",] = "ab"
type T = dict[str, "a" "b"] | Literal["a" "b", "c"]
print >> sys.stderr, "a" "b"
_ = ("a" "b"  # comment
     [0, 1])
with ("a" "b", 1):
    pass
