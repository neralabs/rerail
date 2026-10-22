from collections import namedtuple


class A(namedtuple("A", "x"), \
        object):
    pass


class B(namedtuple("B", "x"), \
        make()):
    pass
