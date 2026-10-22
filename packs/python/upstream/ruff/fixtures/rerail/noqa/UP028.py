def f():
    for x in range(3):
        yield x

def f():
    for x in range(3):  # noqa
        yield x

def f():
    for x in range(3):  # noqa: UP028
        yield x

def f():
    for x in range(3):  # NOQA: UP028
        yield x

def f():
    for x in range(3):  # noqa: E501
        yield x
