class Early:
    @staticmethod
    def s(a, b):
        return a

    @classmethod
    def c(cls, a):
        return a


def factory():
    class Late:
        @staticmethod
        def s(a, b):
            return a

        @classmethod
        def c(cls, a):
            return a

    return Late


staticmethod = lambda fn: fn
classmethod = lambda fn: fn


class After:
    @staticmethod
    def s(a, b):
        return a

    @classmethod
    def c(cls, a):
        return a
