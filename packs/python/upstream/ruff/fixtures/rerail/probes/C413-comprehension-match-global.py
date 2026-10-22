r = [2, 1]
x = [reversed(sorted(r)) for reversed in r]
y = [v for reversed in [r] for v in reversed(sorted(r))]
z = [v for v in reversed(sorted(r)) for reversed in [r]]


def f(v):
    match v:
        case {"k": sorted}:
            pass
    return reversed(sorted(v))


def g():
    global reversed
    reversed = print


def h():
    return reversed(sorted(r)), list(sorted(r))


list = None
