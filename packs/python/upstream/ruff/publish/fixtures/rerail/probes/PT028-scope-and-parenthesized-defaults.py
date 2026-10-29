class TestWithInit:
    def __init__(self): ...

    def test_a(self, a=1): ...


class TestPlain:
    def test_b(self, a=(1), b: int = ((2))):
        def test_inner(c=3): ...

    class Helper:
        def test_c(self, a=4): ...

    class TestNested:
        def test_d(self, a=(x := 6)): ...


class Plain:
    def test_e(self, a=1): ...


if True:

    def test_f(a=(x := 1), *args, b=(1, 2), **kwargs): ...


def testament(a=(  # comment
    5
)): ...
