from typing import Final, Literal

x: Final[
    Literal[
        1  # c
    ]
] = 1

y: Final[Literal[1]] = (
    1
)

z: Final[Literal[  # c1
    "a"
    # c2
]] = "a"; w = 2

v : Final [ Literal [ 1 ] ]   =   1  # trailing
u: Final[Literal[1]] \
    = 1

t: Final[Literal[1]]; s: Final[Literal[2]] = 2
if True: r: Final[Literal[1]] = 1
q: Final[Literal[1,  # c
]] = 1
