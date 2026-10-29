import os
from typing import Annotated, List, Literal

a: "List['os.path.getsize(1)']"
b: "Literal['os.path.getsize(1)']"
c: Annotated[int, "os.path.getsize('f')"]
d: "((os.path.getsize))('f')"
e: "List['List[\"os.path.getsize(filename=1)\"]']"
