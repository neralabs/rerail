def f(x):
    if x:
        return
    return 1

def f(x):
    if x:
        return  # noqa
    return 1

def f(x):
    if x:
        return  # noqa: RET502
    return 1

def f(x):
    if x:
        return  # NOQA: RET502
    return 1

def f(x):
    if x:
        return  # noqa: E501
    return 1
