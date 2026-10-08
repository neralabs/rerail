def outer(x):
    def inner(y):
        def innermost(z):
            return z

        return innermost(y)

    closure = make_adder(x)
    return inner(x) + closure(1)


def make_adder(n):
    def add(m):
        return n + m

    return add
