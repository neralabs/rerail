"""A typing module's member is typing's own (`typingModules = ["std"]`): Ruff's F811 exempts `std.overload`'s
overloads, as it exempts `typing.overload`'s; a module that is no typing module does not."""

from std import overload
import std
import other


@overload
def f(a: int) -> int: ...
@overload
def f(a: str) -> str: ...
def f(a): return a


@std.overload
def g(a: int) -> int: ...
@std.overload
def g(a: str) -> str: ...
def g(a): return a


@other.overload
def h(a: int) -> int: ...
@other.overload
def h(a: str) -> str: ...
def h(a): return a
