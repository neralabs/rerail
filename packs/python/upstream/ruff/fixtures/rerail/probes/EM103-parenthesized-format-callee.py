def a(x):
    raise ValueError(("{}".format)(x))


def b(x):
    raise ValueError(("{}"
                      .format)(x))


def c(x):
    raise ValueError((("{}").format)(x))


def d(x):
    raise ValueError(((("{}")).format(x)))


def e(x):
    raise ValueError("{}".format_map(x))


def f(x):
    raise ValueError("{}"[0].format(x))
