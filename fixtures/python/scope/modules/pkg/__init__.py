class E:
    z = 1


E.z
from . import mod

mod.C.x
from .sub import mod as sub_mod

sub_mod.C.x


def h(a=E(), b=mod.C()):
    pass
