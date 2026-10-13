from contextlib import suppress
def f():
    with (suppress(ValueError)) as cm:
        value = int("1")
    return value
