from typing import Literal
x1: Literal[("a", "b"), "c"] | str
x2: Literal[("a", "b")] | str
x3: Literal["a",] | str
x4: Literal[("a",)] | str
x5: Literal[(("a"))] | str
