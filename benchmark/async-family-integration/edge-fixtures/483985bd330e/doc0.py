import pathlib
from pathlib import Path


async def direct():
    Path("a").open()
    pathlib.Path("a").open()


async def bound():
    p = Path("a")
    p.open()
    q: Path = make()
    q.open()
    r = str("a")
    r.open()
    s = t = pathlib.Path("a")
    s.open()
    t.open()
    with Path("a") as u:
        u.open()
    if (w := Path("a")):
        w.open()
    unknown.open()


async def parameters(annotated: Path, qualified: pathlib.Path, plain):
    annotated.open()
    qualified.open()
    plain.open()


def not_async():
    p = Path("a")
    p.open()


module = Path("a")


async def module_level():
    module.open()
