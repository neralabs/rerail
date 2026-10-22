import os
from os import path as p

(os.path.join)("a", "b")
os.path.join(x for x in ["a", "b"])
os.path.join(*["a"], **{})
p.join()


def shadowed(os):
    return os.path.join("a", "b")


os.path.join.__name__
