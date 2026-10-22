from typing import cast, TypeAlias, Literal
a: "list[__all__.extend(['a', 'a'])]" = 1
b = cast("__all__.extend(['b', \"b\", 'c'])", x)
c: "'__all__.extend([\"c\", \"c\"])'"
d: TypeAlias = "__all__.extend(('d', 'd'))"
type E = "__all__.extend(['e', 'e'])"
f: Literal["__all__.extend(['f', 'f'])"]
g: "(lambda: __all__.extend(['g', 'g']))"
h: "x.__all__.extend(['h', 'h'])"
i: "__all__.extend(['i', 'i'], k)"
j: "__all__.extend(['j', 'j' 'j', 'jj'])"
class K:
    k: "__all__.extend(['k', 'k'])"
def m() -> "__all__.extend(['m', 'm'])": ...
n: "(__all__).extend((['n', ('n')]))"
__all__.extend(\
    ["a", "a"])
__all__.extend(["b", \
    "b"])
__all__ = ["c", \
    "c"]
__all__ += \
    ["d", "d"]
[0 for a[__all__.extend(["Z", "Z"])] in y]
[0 for _ in __all__.extend(["Y", "Y"]) if 1]
{0: 1 for _ in __all__.extend(["X", "X"])}
