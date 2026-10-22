class A:
    @classmethod
    def f(self):
        pass

class A:
    @classmethod
    def f(self):  # noqa
        pass

class A:
    @classmethod
    def f(self):  # noqa: N804
        pass

class A:
    @classmethod
    def f(self):  # NOQA: N804
        pass

class A:
    @classmethod
    def f(self):  # noqa: E501
        pass
