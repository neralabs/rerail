import other

__all__ = [*other.names, "a", other.b]
__all__ += other.more()
x = __all__ = ["b"]


def a(p):
    return p


def b(p):
    return p
