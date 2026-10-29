import py
from typing import Annotated, List

a: "List['py.path.local(1)']"
b: Annotated[int, "py.path.local()"]
c: "((py.path.local))()"
