def a():
    raise \
        ValueError("continued after raise")


def b(x):
    raise \
        ValueError(f"continued {x}")


def c(x):
    raise \
        ValueError("{}".format(x))


def d(x):
    raise \
        (ValueError("parens after continuation"))


def e(x):
    raise ValueError(
        "x") \
        from x
