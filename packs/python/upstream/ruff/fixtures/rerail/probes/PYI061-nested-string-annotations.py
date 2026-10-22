from typing import Literal, Optional

b: "Optional[\"Optional['Literal[None]']\"]"
c: "list['Literal[None, 1]']"
d: "Literal['Literal[None]']"
e: "Optional['''Optional[\"Literal[None]\"]''']"
f: "Optional['''Optional['Literal[None, 1]']''']"
