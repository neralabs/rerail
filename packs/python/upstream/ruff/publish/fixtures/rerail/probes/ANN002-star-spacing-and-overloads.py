from typing import overload


def spaced(
    *  # star comment
    args,
    **  # dstar comment
    kwargs,
):
    pass


@overload
def impl(*args: int) -> int: ...
def impl(*args, **kwargs):
    return 0


class A:
    @overload
    def m(self, *a: int) -> int: ...
    def m(self, *a, **k):
        return 0

    def n(*args, **kwargs):
        return 0


def outer(*args, **kwargs):
    def inner(*a, **k):
        return a
    return lambda *x, **y: x


def only_kw(*, a, **rest):
    pass


def typed_kwargs(*args: "tuple[int, ...]", **kwargs: dict):
    pass
