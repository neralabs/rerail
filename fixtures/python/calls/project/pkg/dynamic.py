import functools

from . import util


def first():
    return 1


def second():
    return 2


HANDLERS = {"first": first, "second": second}


def dispatch(name):
    return HANDLERS[name]()


def call_back(callback):
    return callback()


def by_getattr(name):
    return getattr(util, name)(1)


def via_lambda():
    square = lambda v: v * v  # noqa: E731
    return square(3) + (lambda: first())()


def via_partial():
    return functools.partial(util.helper, 1)()


def exercise():
    dispatch("first")
    dispatch("second")
    call_back(second)
    by_getattr("helper")
    via_lambda()
    via_partial()
