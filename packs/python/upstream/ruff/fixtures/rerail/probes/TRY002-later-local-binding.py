def f():
    raise Exception("a")
    Exception = ValueError


def g():
    def inner():
        raise Exception("b")

    Exception = ValueError
    return inner


def h():
    raise BaseException("c")
    import BaseException
