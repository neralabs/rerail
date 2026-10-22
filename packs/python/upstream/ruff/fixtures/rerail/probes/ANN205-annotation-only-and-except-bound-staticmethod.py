staticmethod: object

try:
    pass
except Exception as staticmethod:
    pass


class A:
    @staticmethod
    def s(): ...

    @staticmethod
    def __new__(cls): ...

    @staticmethod
    @classmethod
    def c(cls): ...

