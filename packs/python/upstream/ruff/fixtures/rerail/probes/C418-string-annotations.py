from typing import List, Literal, cast

a: "dict({})" = 1
b: "dict(" "{})" = 1
c: List["dict({1: 2})"] = []
d: Literal["dict({})"] = 1
e = cast("(dict)({ })", None)


def f(dict):
    x: "dict({})" = 1
    return x
