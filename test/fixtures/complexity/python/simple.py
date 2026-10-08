"""Only simple functions: nothing here is over the limit."""


def add(a, b):
    return a + b


def describe(value):
    if value is None:
        return "nothing"
    return str(value)


def first(items):
    for item in items:
        return item
    return None
