def f(x):
    if x == 'a':
        return 1
    elif x == 'b':
        return 2
    elif x == 'c':
        return 3

def f(x):
    if x == 'a':  # noqa
        return 1
    elif x == 'b':
        return 2
    elif x == 'c':
        return 3

def f(x):
    if x == 'a':  # noqa: SIM116
        return 1
    elif x == 'b':
        return 2
    elif x == 'c':
        return 3

def f(x):
    if x == 'a':  # NOQA: SIM116
        return 1
    elif x == 'b':
        return 2
    elif x == 'c':
        return 3

def f(x):
    if x == 'a':  # noqa: E501
        return 1
    elif x == 'b':
        return 2
    elif x == 'c':
        return 3
