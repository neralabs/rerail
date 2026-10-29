def f():
    a, b = g()  
    return a

def f():
    a, b = g()    # noqa
    return a

def f():
    a, b = g()    # noqa: RUF059
    return a

def f():
    a, b = g()    # NOQA: RUF059
    return a

def f():
    a, b = g()    # noqa: E501
    return a
