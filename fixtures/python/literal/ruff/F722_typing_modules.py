"""A typing module's member is typing's own (`typingModules = ["std"]`): Ruff's F722 skips a function decorated with
`std.no_type_check`, as with `typing.no_type_check`; a module that is no typing module does not."""

import std
import other
from typing import no_type_check


@std.no_type_check
def f(x: "a b") -> None: ...


@no_type_check
def g(x: "a b") -> None: ...


@other.no_type_check
def h(x: "a b") -> None: ...
