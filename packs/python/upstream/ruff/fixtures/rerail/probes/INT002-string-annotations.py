from typing import List, Literal, cast
a = 1
t = "{}"
x1: "_(t.format(a))" = 1
x2: "List['gettext(t.format(a))']" = 1
x3: "_((t).format(a))" = 1
x4: "_(format.join(a))" = 1
x5: "_(t.format)" = 1
x6: "Literal['_(t.format(a))']" = 1
y = cast("_(a, t.format(a))", 1)
def f(p: "ngettext(t.format(a), '', 1)") -> None: ...
