from typing import Dict
xs = range(3)
a: "list(x for x in xs)"
b: "Dict['list(2 * x for x in xs)', int]"
c = "list(x for x in xs)"


def f(list: int) -> "list(x for x in xs)":
    d: "list(x for x in xs)"
    return list


class C:
    list = tuple
    e: "list(x for x in xs)"
