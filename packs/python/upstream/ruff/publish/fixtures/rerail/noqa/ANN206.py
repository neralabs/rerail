class C:
    @classmethod
    def f(cls):
        pass

class C:
    @classmethod
    def f(cls):  # noqa
        pass

class C:
    @classmethod
    def f(cls):  # noqa: ANN206
        pass

class C:
    @classmethod
    def f(cls):  # NOQA: ANN206
        pass

class C:
    @classmethod
    def f(cls):  # noqa: E501
        pass
