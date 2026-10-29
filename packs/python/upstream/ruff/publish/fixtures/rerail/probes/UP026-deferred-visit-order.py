def f(a=mock.mock.X, b: mock.mock.Y = None):
    return mock.mock.Z


g = lambda a=mock.mock.X: mock.mock.Y
h: "mock.mock.X"


def outer():
    def inner():
        return (mock).mock.X
    import mock
    return inner
