def f(x):
    if x:
        return True
    else:
        return False

def f(x):
    if x:  # noqa
        return True
    else:
        return False

def f(x):
    if x:  # noqa: SIM103
        return True
    else:
        return False

def f(x):
    if x:  # NOQA: SIM103
        return True
    else:
        return False

def f(x):
    if x:  # noqa: E501
        return True
    else:
        return False
