import typing
import typing as t
import typing_extensions
import typing_extensions as te
from typing import overload as ov
from typing_extensions import overload as ov2
from _typeshed import overload as ov3


@typing.overload
def a(x: int) -> int:
    """Doc."""


@t.overload
def b(x: int) -> int:
    """Doc."""


@typing_extensions.overload
def c(x: int) -> int:
    """Doc."""


@te.overload
def d(x: int) -> int:
    """Doc."""


@ov
def e(x: int) -> int:
    """Doc."""


@ov2
def g(x: int) -> int:
    """Doc."""


@ov3
def h(x: int) -> int:
    """Doc."""
