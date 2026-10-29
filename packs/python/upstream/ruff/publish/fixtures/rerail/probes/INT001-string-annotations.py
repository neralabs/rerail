from typing import List, Literal, TypeVar, cast
import gettext as g

a = 1
x1: "_(f'{a}')" = 1
x2: "g.gettext(f'{a}' 'b')" = 1
x3: "List[_(f'{a}')]" = 1
x4: "Literal['_(f\"{a}\")']" = 1
x5: f"_('{a}')" = 1
T = TypeVar("T", bound="_(a, f'{a}')")
y = cast("ngettext(f'{a}', 'x', 1)", 1)
def f() -> "_(\"%s\" f'{a}')": ...
