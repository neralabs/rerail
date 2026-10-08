"""Locals read only from string type definitions, which read the module's names first."""

from typing import List, cast

shadowed = 1


def reads_a_local_in_a_string() -> None:
    alias = int
    value: "alias" = 1
    return value


def reads_the_module_first() -> None:
    shadowed = 2
    value: "shadowed" = 1
    return value


def reads_in_a_cast(value: object) -> None:
    kind = str
    return cast("kind", value)


def reads_in_a_generic() -> None:
    element = int
    return List["element"]


def reads_in_a_literal() -> None:
    unused = 1
    from typing import Literal

    value: Literal["unused"] = "unused"
    return value


def bare_annotations() -> None:
    declared: int
    read: int
    print(read)
    later: "declared"
