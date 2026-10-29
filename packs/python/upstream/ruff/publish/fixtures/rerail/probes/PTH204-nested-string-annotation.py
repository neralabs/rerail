import os
from typing import Annotated, List, Literal

a: "List['os.path.getmtime(1)']"
b: "Literal['os.path.getmtime(1)']"
c: Annotated[int, "os.path.getmtime('f')"]
d: "((os.path.getmtime))('f')"
e: "List['List[\"os.path.getmtime(filename=1)\"]']"
