# target-version: py314
T = int


def annotates():
    y: T = 1
    T = 1
    return y


def annotates_bare():
    z: T
    T = 2
    return T


def annotates_then_reads():
    y: T = 1
    print(T)
    T = 1
    return y


def annotates_a_builtin():
    y: int = 1
    int = 2
    return y


def outer():
    U = int

    def inner():
        y: U = 1
        U = 1
        return y

    return inner, U
