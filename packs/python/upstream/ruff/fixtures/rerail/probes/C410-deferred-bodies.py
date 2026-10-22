from __future__ import annotations


def f(a: list([1])) -> list((1,)):
    return a


g = lambda: list([1])
h = lambda a=list((1,)): a


def outer():
    def inner():
        return list([1])

    list = None
    return inner


list = None
