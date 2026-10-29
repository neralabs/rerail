classmethod: object

try:
    pass
except Exception as classmethod:
    pass


class A:
    @classmethod
    def c(cls): ...

    @classmethod
    @staticmethod
    def d(cls): ...
