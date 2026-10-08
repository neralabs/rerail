class C:
    x = 1


C.x
from . import sibling

sibling.attr


def f(a=C(), b=sibling()):
    pass
