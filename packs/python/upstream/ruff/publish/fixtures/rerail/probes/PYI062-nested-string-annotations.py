from typing import Literal, Optional

a: "Optional['Literal[1, 1]']"
b: "list['''Optional[\"Literal['x', 'x']\"]''']"
c: "Literal['Literal[1, 1]', 2]"
