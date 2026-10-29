def a() -> None:
    r"""raw doc"""
def b() -> None:
    u"""unicode doc"""
def c() -> None:
    R'''raw single'''
def d() -> None:
    b"""bytes not doc"""
def e() -> None:
    f"""fstring not doc"""
def g(x: int) -> None:
    f"{x}"
def h() -> None:
    "implicit" "concat"
def i() -> None:
    "implicit" f"fconcat"
def j() -> None:
    rb"bytes raw"
def k() -> None:
    U"upper unicode"
def l() -> None:
    f"" "x"
def m() -> None:
    '''a''' '''b'''
    ...
def a() -> None:
    ("""parenthesized doc""")
def b() -> None:
    (("double paren"))
def c() -> None:
    ("implicit"
     "concat")
def d() -> None:
    (
        # comment inside
        "doc with comment"
    )
def e() -> None:
    ("a", "b")
def f() -> None:
    ("a" + "b")
def g() -> None:
    ("""x""").upper()
def a() -> None: """one-liner doc"""
class B: """one-liner class doc"""
def c() -> None: """doc"""; ...
def d() -> None: ...; """not doc"""
class E: x: int; """not doc"""
def f(): "single quote doc"
