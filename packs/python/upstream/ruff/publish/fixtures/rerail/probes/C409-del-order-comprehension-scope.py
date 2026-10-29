r = [2, 1]
p = [tuple for tuple in tuple([1])]
q = [y for tuple in r for y in tuple([1])]


def d1(c):
    if c:
        del tuple
    return tuple([1])


def d2(r):
    for _ in r:
        del tuple
    return tuple([1])


def d3():
    try:
        pass
    except NameError:
        del tuple
    return tuple((1,))


def d4():
    tuple: type = tuple([1])
    return tuple
