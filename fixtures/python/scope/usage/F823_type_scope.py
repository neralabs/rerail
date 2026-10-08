# target-version: py313
T = int


def base():
    class C(T):
        pass

    T = 1


def keyword():
    class C(metaclass=T):
        pass

    T = 1


def parameter():
    def g(a: T):
        pass

    T = 1


def variadic():
    def g(*a: T, **k: T):
        pass

    T = 1


def returns():
    def g() -> T:
        pass

    T = 1


def generic():
    def g[U](a: T) -> U:
        pass

    class C[U](T):
        pass

    T = 1


def default():
    def g(a=T):
        pass

    T = 1


def decorator():
    @T
    def g():
        pass

    T = 1


def base_then_body():
    class C(T):
        pass

    print(T)
    T = 1
