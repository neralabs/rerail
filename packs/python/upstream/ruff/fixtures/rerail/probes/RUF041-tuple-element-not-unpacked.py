from typing import Literal

a = Literal[(1, Literal[2, Literal[3]]), 4]
b = Literal[(Literal[Literal[1]],), 2]
c = Literal[(Literal[1], 2), 3]
d = Literal[(1, (Literal[2, Literal[3]])), 4]
e: Literal[(1, Literal[2, Literal[3]]), 4] = 1
f: Literal[(Literal[Literal[1]],), 2] = 1
g = Literal[(1, Literal[2, Literal[3]]),]
h = Literal[Literal[1],]
i: Literal[(1, Literal[2, Literal[3]]),] = 1
j: "Literal[(1, Literal[2, Literal[3]]), 4]" = 1
