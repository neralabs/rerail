def f(a: A, b: t.Any, c: Opt, d: t.Union[int, A], e: Late) -> Opt[A]: ...


class K:
    def m(self, x: A, y: t.Optional) -> t.Any: ...


def setup():
    from typing import Any as Local
    return Local


def g(a: Local): ...


from typing import Any as A, Optional as Opt
from typing import Any as Late
import typing as t

Late = object
