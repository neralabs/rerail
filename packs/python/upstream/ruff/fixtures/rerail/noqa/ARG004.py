class C:
    @staticmethod
    def f(x):
        return 1

class C:
    @staticmethod
    def f(x):  # noqa
        return 1

class C:
    @staticmethod
    def f(x):  # noqa: ARG004
        return 1

class C:
    @staticmethod
    def f(x):  # NOQA: ARG004
        return 1

class C:
    @staticmethod
    def f(x):  # noqa: E501
        return 1
