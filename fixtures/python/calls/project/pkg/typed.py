"""Receivers whose class the program can name: builtin values and workspace instances held in names."""
from pkg.models import Base, Child


class Shape:
    def area(self):
        return 0

    def scaled(self):
        return Square()


class Square(Shape):
    def area(self):
        return 4


def make_shape(square):
    if square:
        return Square()
    return Shape()


def exercise():
    text = str(12)
    text.upper()
    data = b"ab"
    data.decode()
    words = ["a", "b"]
    words.append("c")
    table = {"a": 1}
    table.get("a")
    "-".join(words)
    child = Child(1)
    child.describe()
    # Every path runs at each site, so the trace holds the union the program gives.
    for square in (True, False):
        shape = make_shape(square)
        shape.area()
    Shape().scaled().area()
    for first in (True, False):
        base = Base(2) if first else child
        base.describe()
