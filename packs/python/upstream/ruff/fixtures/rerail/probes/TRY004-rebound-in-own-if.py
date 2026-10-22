def f(x):
    if isinstance(x, int):
        raise ValueError("a")
        isinstance = 1


def g(x):
    if callable(x):
        raise ValueError("b")
    else:
        callable = 2


def f1(x):
    if isinstance(x, int):
        raise ValueError("nested def name")
        def isinstance(): ...


def f2(x):
    if callable(x):
        raise ValueError("nested def param")
        def g(callable): ...


def f3(x):
    if issubclass(x, int):
        raise ValueError("lambda param")
        h = lambda issubclass: 1


def f4(x, y):
    if isinstance(x, int):
        raise ValueError("comprehension var")
        z = [isinstance for isinstance in y]


def f5(x):
    if callable(x):
        raise ValueError("for target")
    else:
        for callable in x:
            pass


def f6(x):
    if issubclass(x, int):
        raise ValueError("import")
    elif isinstance(x, int):
        from m import issubclass


def f8(x):
    if isinstance(x, int):
        raise ValueError("nested class")
        class isinstance: ...


def f9(x):
    if callable(x):
        raise ValueError("other name")
        isinstance = 1
