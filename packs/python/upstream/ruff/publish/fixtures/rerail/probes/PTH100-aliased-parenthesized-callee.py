import os
from os import path as pp

pp.abspath("a")
(os.path.abspath)("b")
os.path.abspath(path="c")
os.path.abspath(x for x in ["d"])
os.path.abspath("e", "f")
os.path.abspath(*["g"])
os.path.abspath(**{"path": "h"})
os.path.abspath(name="i")


def shadowed(os):
    os.path.abspath("j")
