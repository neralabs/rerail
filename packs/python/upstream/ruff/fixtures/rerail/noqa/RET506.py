def f(x):
    if x:
        raise ValueError
    else:
        return 2

def f(x):
    if x:
        raise ValueError
    else:  # noqa
        return 2

def f(x):
    if x:
        raise ValueError
    else:  # noqa: RET506
        return 2

def f(x):
    if x:
        raise ValueError
    else:  # NOQA: RET506
        return 2

def f(x):
    if x:
        raise ValueError
    else:  # noqa: E501
        return 2
