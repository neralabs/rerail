"""Doc."""
__all__ = 'Foo', ('f')
__all__ += (('Zed')),


class Foo:
    def m(self): ...


class Zed:
    def m(self): ...


class Other:
    def m(self): ...


def f(): ...


def h(): ...
