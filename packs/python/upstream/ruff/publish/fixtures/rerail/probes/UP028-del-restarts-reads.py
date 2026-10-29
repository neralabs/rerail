def f():
    for x in y:
        yield x
    print(x)
    del x
    x = 1


def g():
    for x in y:
        yield x
    if c:
        del x
    x = 1


def h():
    for x in y:
        yield x
    del (x)
    x = 1
    print(x)


def i():
    for x in y:
        yield x
    del x
    del x
