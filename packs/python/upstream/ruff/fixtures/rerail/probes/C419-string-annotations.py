from typing import List, Literal, cast

a: "any([x for x in y])" = 1
b: "any(" "[x for x in y])" = 1
c: List["all({x for x in y})"] = []
d: Literal["any([x for x in y])"] = 1
e = cast("all([x for x in y], 1)", None)


def f(any):
    x: "any([z for z in y])" = 1
    return x
