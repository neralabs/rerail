from os import *

handled = None


def reads_then_binds():
    print(len)
    print(len)
    len = 1


def binds_then_reads():
    sum = 1
    print(sum)


def binds_twice():
    print(min)
    min = 1
    min = 2


def augments():
    max += 1


def outer():
    def inner():
        print(abs)
        abs = 1

    return inner


def reads_in_a_comprehension():
    [ord for _ in ()]
    ord = 1


def declares_global():
    global chr
    print(chr)
    chr = 1


def reads_a_magic_global():
    print(__name__)
    __name__ = "x"


def reads_a_name_no_builtin_has():
    print(sep)
    sep = 1


def catches_later():
    print(hash)
    try:
        pass
    except Exception as hash:
        pass


def catches_a_module_name_later():
    print(handled)
    try:
        pass
    except Exception as handled:
        pass


def imports_later():
    print(open)
    from io import open


class Holder:
    def method(self):
        print(iter)
        iter = 1
