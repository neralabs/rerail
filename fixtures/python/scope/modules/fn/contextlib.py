def suppress(*args):
    pass


class Box:
    def open(self):
        pass


with suppress():
    pass
Box.open


def f(a=suppress(), b=Box()):
    class Box:
        pass

    Box.open
    return suppress


def g(c=len()):
    pass
