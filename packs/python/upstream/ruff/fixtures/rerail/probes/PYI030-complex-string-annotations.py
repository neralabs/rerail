from typing import Literal
a: "Literal['a'] | Literal[\"b\"]"
b: "Literal[1] " "| Literal[2]"
c: "Literal[1] | Literal[2] | int" ""
d: "list[int] | Literal[1]" ""
e: "Literal[1] | Literal[2]"
