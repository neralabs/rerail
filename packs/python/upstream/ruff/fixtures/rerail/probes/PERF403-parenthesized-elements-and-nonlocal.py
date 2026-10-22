def made() -> dict:
    return {}


def f(items):
    with dict() as res:
        for k, v in items:
            res[k] = v
    for (k), (v) in items:
        made[k] = v
    out = {}
    for ((k)), ((v, w)) in items: out[k] = v, w;
    acc = {}
    for a, b in items:
        acc[a] = b
    def g():
        nonlocal b
        b = 0
    return g
