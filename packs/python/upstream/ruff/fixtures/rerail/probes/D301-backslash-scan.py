"""Module \d doc."""


def a():
    "\\"


def b():
    "a\"b"


def c():
    """a\""" b"""


def d():
    '''a\''' b'''


def e():
    """a\"\"\" b"""


def f():
    "\N{EM DASH} ok é \U0001F600"


def g():
    "é then \d"


def h():
    "\é"


def i():
    u"\d"


def j():
    R"\d"


def k():
    """a\"" b"""


def l():
    """a\"\"x"""


def m():
    """line \
continued"""


def n():
    "\x41"


def o():
    'a\'b'


def p():
    """a\"\""""


def q():
    rb"\d"


def r():
    b"\d"


def s():
    x = 1
    "\d"


def t():
    "\d" "x"


def u():
    ("\d")


def v():
    "\\\\"


def w():
    """\'\'\' tail"""


def x():
    """\'\'"""


def y():
    "\
"
