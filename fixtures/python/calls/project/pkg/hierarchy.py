"""Dispatch the native resolver decides and the typing program must match: a metaclass `__call__`, a `__new__`,
and a method found by the C3 linearization (D's MRO is D, B, C, A: `who` is C's, not A's)."""


class Meta(type):
    def __call__(cls, *args):
        return super().__call__(*args)


class Made(metaclass=Meta):
    def __init__(self, x):
        self.x = x


class WithNew:
    def __new__(cls, x):
        return super().__new__(cls)

    def __init__(self, x):
        self.x = x


class A:
    def who(self):
        return "A"


class B(A):
    pass


class C(A):
    def who(self):
        return "C"


class D(B, C):
    pass


def exercise():
    Made(1)
    WithNew(2)
    D.who(D())
