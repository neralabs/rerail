handle = Outer()


def make() -> Outer: ...


class Outer:
    def m(self):
        return handle._a, make._b

    def __eq__(self, other):
        f = lambda a=other._c: a._d
        class In(other._e):
            pass
        return f, In


handle._f
