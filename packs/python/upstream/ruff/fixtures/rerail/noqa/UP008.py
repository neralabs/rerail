class A:
    pass


class B(A):
    def f(self):
        super(B, self).f()

class A:
    pass


class B(A):
    def f(self):
        super(B, self).f()  # noqa

class A:
    pass


class B(A):
    def f(self):
        super(B, self).f()  # noqa: UP008

class A:
    pass


class B(A):
    def f(self):
        super(B, self).f()  # NOQA: UP008

class A:
    pass


class B(A):
    def f(self):
        super(B, self).f()  # noqa: E501
