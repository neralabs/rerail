from typing import TypeAlias, TYPE_CHECKING

if TYPE_CHECKING:
    from foo import Foo, Early, Late

Early = 1


def f():
    class Local: ...
    a: TypeAlias = Local | Early | Late | list


def outer():
    Foo = 3
    def inner():
        b: TypeAlias = Foo
    return inner


class K:
    Bar = int
    c: TypeAlias = Bar


Late = 2
