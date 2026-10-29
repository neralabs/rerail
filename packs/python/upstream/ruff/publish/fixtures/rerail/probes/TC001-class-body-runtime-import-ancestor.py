class A:
    import pkga
    import os
    from . import rel
    v = pkga.thing
    w = os.sep
    u = rel.x

    def m(self):
        from pkga import B
        from os import path
        from . import other
        y: B
        z: path
        q: other


class C:
    import pkgc
    c = pkgc.x


def g():
    from pkgc import D
    d: D
