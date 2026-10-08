n: int
print(n)


def f():
    print(n)


def g():
    global n


print(n)

m: int


def h():
    global m
    m = 1


def k():
    return m


o: int


def p():
    def q():
        global o

    return q


def r():
    return o


s: int


class C:
    global s


def t():
    return s
