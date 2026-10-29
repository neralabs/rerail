class C:
    @classmethod
    def f(cls, x):
        return 1

class C:
    @classmethod
    def f(cls, x):  # noqa
        return 1

class C:
    @classmethod
    def f(cls, x):  # noqa: ARG003
        return 1

class C:
    @classmethod
    def f(cls, x):  # NOQA: ARG003
        return 1

class C:
    @classmethod
    def f(cls, x):  # noqa: E501
        return 1
