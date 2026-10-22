a1: list[int] | str | None = None
a3: list[int] | None | str = None
a7: list[int] | str | float | None = None
a12: list[int] | None | str | bytes = None
b1: list[int] | None | dict[str, int] = None
b2: dict[str, list[int]] | (str | None) | None = None
b3: list[int] | str | dict[str, int] | None = None
b4: (
    list[int]  # c
    | str
    | None
) = None
b5: list[int] | Literal["a|b", 'c'] | None = None
