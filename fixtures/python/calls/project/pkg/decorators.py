import functools


def plain(func):
    func.marked = True
    return func


def wrapping(func):
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        return func(*args, **kwargs)

    return wrapper


@plain
def marked():
    return 1


@wrapping
def wrapped():
    return 2


@functools.lru_cache(maxsize=None)
def cached(x):
    return x


def exercise():
    marked()
    wrapped()
    cached(1)
