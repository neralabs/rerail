from __future__ import annotations
import os.path
import a.b as ab
from ..pkg import thing as t, other
from . import sib

__all__ = ["run", 'Klass', "x" "y"]

if True:
    import late

def helper(x, *args, y=1, **kw):
    z = x
    local_fn()
    def inner():
        return getattr(ctx, "materialized_pipeline")
    inner()
    os.path.join(z)
    t()
    sib.go()
    return z

@legacy_async
@pkg.deco(1)
async def run(p: int):
    global gg
    gg()
    helper(p)
    for q in p: q()
    ab.c.f()

class Klass(Base):
    def m(self):
        self.n()
        Klass.m2()
        execute_runtime_nested_steps_sync()
    def n(self):
        import zz
        zz.q()
        return self.ctx.materialized_pipeline

def local_fn(): pass
