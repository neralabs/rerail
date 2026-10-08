class C:
    x = 1


C.x


def f():
    class D:
        y = 2

    D.y


from . import sibling

sibling.attr
from .. import top

top.attr
from ... import beyond

beyond.attr
from .... import far

far.attr
from .sibling import thing, make

thing.attr


def k(a=make(), b=f(), c=C(), d=far()):
    pass
