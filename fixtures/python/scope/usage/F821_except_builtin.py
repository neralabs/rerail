try:
    pass
except Exception as frozenset:
    pass
print(frozenset)


def reads_it_later():
    return frozenset


def catches_a_builtin():
    try:
        pass
    except Exception as len:
        pass
    return len


def catches_in_an_enclosing_function():
    try:
        pass
    except Exception as sum:
        pass

    def inner():
        return sum

    return inner


def catches_a_name_no_builtin_has():
    try:
        pass
    except Exception as missing:
        pass
    return missing


try:
    pass
except Exception as __name__:
    pass
print(__name__)
