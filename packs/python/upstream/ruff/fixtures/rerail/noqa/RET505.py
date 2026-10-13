def f(x):
    if x:
        return 1
    else:
        return 2

def f(x):
    if x:
        return 1
    else:  # noqa
        return 2

def f(x):
    if x:
        return 1
    else:  # noqa: RET505
        return 2

def f(x):
    if x:
        return 1
    else:  # NOQA: RET505
        return 2

def f(x):
    if x:
        return 1
    else:  # noqa: E501
        return 2
