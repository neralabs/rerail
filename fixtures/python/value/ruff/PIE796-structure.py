# Structural equality (Ruff's ComparableExpr): a repeated enum value of the same type, spelled any way.
import enum


class A(enum.Enum):
    a = 1
    b = 0x1
    c = 1.0
    d = True
    e = 1_0
    f = 10
    g = 0j
    h = 0
    i = "a"
    j = 'a'
    k = "a" "b"
    l = "ab"
    m = b"ab"
    n = "\x61"
    o = 1e-400
    p = 0.0
    q = -0.0
    r = -0.0
    s = 0xffffffffffffffffffff
    t = 1208925819614629174706175
    u = 18446744073709551616
    v = 18446744073709551616
    w = (1)
    x = f"a"
    y = f"a"
    z = (1, "a")
    aa = (1, 'a')
    ab = 1.5
    ac = 1.50
    ad = None
    ae = None
    af = 1e400
    ag = 2e400
    ah = -(1)
    ai = -1
    aj = f"{x}"
    ak = f'{x}'
    al = "\N{DIGIT ONE}"
    am = "1"
    an = 1j
    ao = 1.0j
    ap = [1, 2]
    aq = [1,2]
    ar = [1, 0x2]
    at = [1, (2, 'b')]
    au = [1, (2, "b")]
    av = f"{x!r:>{w}}"
    aw = f'{x!r:>{w}}'
    ax = f"{x=}"
    ay = f"{x = }"
    az = f"{x:}"
    ba = f"{x}"
    bb = a.b(c, k=1)
    bc = a . b(c, k=(1))
    bd = x[1:2]
    be = x[1:2:]
    bf = x[1,]
    bg = x[(1,)]
    bh = lambda y, *, z=1: y
    bi = lambda y, *, z=0x1: y
    bj = 0xffffffffffffffffffff
    bk = 0XFFFFFFFFFFFFFFFFFFFF
    bl = 18_446_744_073_709_551_616
    bm = 18446744073709551616
    bn = "\N{LATIN SMALL LETTER A}" "b"
    bo = "ab"
    bp = not a
    bq = not (a)
    br = a and b and c
    bs = (a and b) and c
    bt = a and (b and c)
    bu = a < b <= c
    bv = (a < b) <= c
    bw = {1: 2, **d}
    bx = {1: 2, **(d)}
    by = f(*a, k=1)
    bz = f(k=1, *a)
