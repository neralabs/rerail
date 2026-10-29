class C:
    @staticmethod
    def f():
        pass

class C:
    @staticmethod
    def f():  # noqa
        pass

class C:
    @staticmethod
    def f():  # noqa: ANN205
        pass

class C:
    @staticmethod
    def f():  # NOQA: ANN205
        pass

class C:
    @staticmethod
    def f():  # noqa: E501
        pass
