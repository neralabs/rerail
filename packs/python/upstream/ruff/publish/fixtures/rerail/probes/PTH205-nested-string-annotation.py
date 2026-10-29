import os
from typing import Annotated, List, Literal

a: "List['os.path.getctime(1)']"
b: "Literal['os.path.getctime(1)']"
c: Annotated[int, "os.path.getctime('f')"]
d: "((os.path.getctime))('f')"
e: "List['List[\"os.path.getctime(filename=1)\"]']"
