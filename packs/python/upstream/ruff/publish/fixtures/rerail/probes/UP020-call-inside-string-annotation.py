import io
import typing

x: "io.open('f')"
y = typing.cast("io.open('f')", 1)


def f(a: "io.open('a')") -> "list[io.open('b')]":
    pass
