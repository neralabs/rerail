import os
from typing import Annotated, List, Literal

a: "List['os.path.getatime(1)']"
b: "Literal['os.path.getatime(1)']"
c: Annotated[int, "os.path.getatime('f')"]
d: "((os.path.getatime))('f')"
e: "List['List[\"os.path.getatime(filename=1)\"]']"
