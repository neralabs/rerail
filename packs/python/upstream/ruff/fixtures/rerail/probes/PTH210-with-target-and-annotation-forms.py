from pathlib import Path


def f(p: (Path)):
    p.with_suffix("py")


with Path() as s, Path() as (t, u):
    s.with_suffix("py")
    t.with_suffix("py")
r: (Path) = x
r.with_suffix("py")
