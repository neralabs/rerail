class C:
    def a(self, x, /, y, *, z, **kw): print(1)
    def b(self, /, x): print(1)
    def c(*, x): print(1)
    def d(self, *, x): print(1)
    def e(self, *, x=1, y=2): print(1)
    def f(self=None, x=1): print(1)
    def g(self: "C", *args: int, **kwargs: str): print(1)
    def h(self, x=lambda y: 0): print(1)
    def i(*args, x): print(1)
    def j(*args, **kwargs): print(1)
    def k(**kwargs): print(1)

    @classmethod
    def ca(cls, /, x, *, y): print(1)

    @classmethod
    def cb(*, x): print(1)

    @classmethod
    def cc(*args, x): print(1)

    @classmethod
    def cd(): print(1)

    @staticmethod
    def sa(x, /, y, *, z, **kw): print(1)

    @staticmethod
    def sb(*, x): print(1)

    @staticmethod
    def sc(): print(1)

    def __new__(cls, /, x, *, y): print(1)

    def __new__(*, x): print(1)

    def __init_subclass__(cls, /, x, *, y): print(1)

    def __class_getitem__(cls, x, y): print(1)


def fn(a, /, b, *, c): print(1)
def fn2(*, a): print(1)
def fn3(a=1, /, b=2, *c, d=3, **e): print(1)
lam1 = lambda a, /, b, *, c: 0
lam2 = lambda *, a: 0
lam3 = lambda a=1, /, b=2, *c, d=3, **e: 0
