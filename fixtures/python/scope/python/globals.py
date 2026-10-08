import time


def reads_before_declaring():
    return declared_later


declared_later


def declares():
    global declared_later, time
    time = 3
    return declared_later, time


declared_later


def declares_only():
    global never_assigned
    return never_assigned


never_assigned


def reads_import():
    return time


class Holder:
    global class_declared
    class_declared = 1
    value = class_declared


class_declared


def outer():
    def inner():
        global nested_declared
        nested_declared = 1

    return inner


nested_declared


def reads_nested():
    return nested_declared


print(time, len)


def declares_builtin():
    global len
    len = 1
    return len


len
