__all__ = list(["a"], ["b"])
__all__ += list(*[["c"]])
__all__ += tuple(["d"], key=1)
__all__ += tuple((["e"]))
__all__ += list()
__all__ += sorted(["f"])
y = 1
__all__ += [y, "g"]
__all__ = __all__ + ["h"]


def a(): pass
def b(): pass
def c(): pass
def d(): pass
def e(): pass
def f(): pass
def g(): pass
def h(): pass
