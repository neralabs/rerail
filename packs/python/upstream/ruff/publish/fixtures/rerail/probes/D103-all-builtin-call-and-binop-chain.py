"""Doc."""
__all__ = tuple({"Foo", ("f")}) + list(["Zed"]) + ["g"]


class Foo:
    def m(self): ...


class Zed:
    def m(self): ...


class Other:
    def m(self): ...


def f(): ...


def g(): ...


def h(): ...
