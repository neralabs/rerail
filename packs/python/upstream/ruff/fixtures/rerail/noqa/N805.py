class A:
    def f(x):
        pass

class A:
    def f(x):  # noqa
        pass

class A:
    def f(x):  # noqa: N805
        pass

class A:
    def f(x):  # NOQA: N805
        pass

class A:
    def f(x):  # noqa: E501
        pass
