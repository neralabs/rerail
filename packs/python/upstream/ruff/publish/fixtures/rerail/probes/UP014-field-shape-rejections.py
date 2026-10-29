from typing import NamedTuple as NT
T = NT("T", [("a", int, 3)])
W = NT("W", [("__x__", int)])
X = NT("X", [("_x", int)])
Z = NT("Z", [("a", *x)])
a1 = NT("a1", [["a", int]])
a4 = NT("a4", ())
b2 = NT("b2", [("a", int)], a=int)
if x: c5 = NT("c5", [("a", int)])
