import builtins
type("", x=1)
type(("a"))
type(  # c
    0j
)
type(1, 2)
type(*"a")
type(f"a")
type("a" f"b")
type(b"a" b"c")
type(True)
type(None)
type(...)
type(-1)
type(0x1F)
type(1.5e3J)
builtins.type(1)
(type)(1)
type(1,)
type(\
1)
type(1)(2)
def f(type):
    type(1)
