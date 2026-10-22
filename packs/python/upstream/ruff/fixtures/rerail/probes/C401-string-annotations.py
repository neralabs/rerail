from typing import Dict
xs = range(3)
a: "set(x for x in xs)"
b: "Dict['set(2 * x for x in xs)', int]"
c = "set(x for x in xs)"


def f(set: int) -> "set(x for x in xs)":
    d: "set(x for x in xs)"
    return set


class C:
    set = tuple
    e: "set(x for x in xs)"
