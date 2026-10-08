# target-version: py314
def f(x: Later, *, y: Later = default) -> Later2:
    z: Local = 1
    return z


w: Later3 = 1


class C:
    a: Later4
    b: Later5 = default

    def m(self, v: Later6) -> Later7:
        pass


lam = lambda q: q
Later = Later2 = Later3 = Later4 = Later5 = Later6 = Later7 = Local = int
default = None
