"""Names in string type definitions: forward references, exact and whole-string offsets."""

from typing import Dict, List, Literal, Optional, cast


def forward(value: "Later") -> "Optional[Missing]":
    return value


class Later:
    field: "Later"
    other: "Dict[str, 'Nested']"
    escaped: "List[\x55ndefined]"
    concatenated: "List" "[Joined]"
    literal: Literal["NotAName"]
    triple: """
        Optional[
            Spread
        ]
    """


def casts(value: object) -> None:
    cast("Casted", value)
    cast("Later", value)


undefined_call = cast
print(undefined_call("NotAType", 1))
