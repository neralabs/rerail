"""Typing settings (`typingModules = ["compat"]`, `extendGenerics = ["lib.Box"]`): Literal, generics, typing callables, a
TypeAlias, and in a stub the type definitions read after the module."""

from compat import Literal, Optional, TypeAlias, cast, TypeVar
from lib import Box

x: Literal["a"] = "a"
y = Optional["Undefined"]
Z: TypeAlias = "Undef2"
cast("Undef3", 1)
T = TypeVar("T", bound="Undef4")
w = Box["Undef5"]
v = Optional[Later]
u = Box[Later2]


class Later: ...
class Later2: ...
