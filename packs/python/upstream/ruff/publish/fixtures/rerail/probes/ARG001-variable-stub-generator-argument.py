def a(x):
    obj.msg = "not implemented"
    raise NotImplementedError(i for i in ())


def b(x):
    msg = "not implemented"
    raise NotImplementedError(i for i in ())


def c(x, y=lambda z: locals()):
    obj.msg = f"{y}"
    raise NotImplementedError(*y)
