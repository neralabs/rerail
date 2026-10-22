from typing import List


def made() -> List[int]:
    return []


def f(items):
    with list() as res:
        for i in items:
            res.append(i + 1)
    for i in items:
        made.append(i + 1)
    out = []
    for ((i)) in items: out.append(i + 1);
    acc = []
    for j in items:
        acc.append(j + 1)
    def g():
        nonlocal j
        j = 0
    return g
