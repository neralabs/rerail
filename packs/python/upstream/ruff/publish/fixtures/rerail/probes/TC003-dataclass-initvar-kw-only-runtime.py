from __future__ import annotations

from dataclasses import InitVar
from dataclasses import KW_ONLY
import pkgp


def make():
    from dataclasses import dataclass

    @dataclass
    class A:
        a: InitVar[pkgp.X]
        _: KW_ONLY
        b: int

    return A
