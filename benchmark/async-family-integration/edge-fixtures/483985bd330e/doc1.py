import httpx
from pathlib import Path


async def unpacked():
    a, p = 1, Path("x")
    p.open()
    a.open()
    (p1, p2) = (Path(), Path())
    p1.open()
    p2.open()
    [l1, l2] = [Path(), 2]
    l1.open()
    l2.open()
    n, (m, o) = 0, (Path(), Path())
    m.open()
    o.open()
    q, r = s, t = Path(), 3
    q.open()
    s.open()
    r.open()
    (u), v = (Path()), 4
    u.open()
    w, *rest, z = Path(), 5, Path()
    w.open()
    z.open()
    rest.open()
    b, *more = Path(), 6, 7
    b.open()
    c, d = pair()
    c.open()
    e, f = *pair(), Path()
    e.open()
    g, (h, i) = Path(), pair()
    g.open()
    h.open()
    if (j := (Path(), 1)):
        j.open()
    x = y, yy = Path(), 8
    y.open()
    x.open()
    for loop, other in [(Path(), 1)]:
        loop.open()
    with Path() as (wa, wb):
        wa.open()
    single, = Path(),
    single.open()


def not_async():
    a, p = 1, Path("x")
    p.open()


module_a, module_b = Path(), 1


async def module_level():
    module_a.open()
    module_b.open()
