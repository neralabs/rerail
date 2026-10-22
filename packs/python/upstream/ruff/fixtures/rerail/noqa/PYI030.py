from typing import Literal
def f(x: Literal[1] | Literal[2]): ...

from typing import Literal
def f(x: Literal[1] | Literal[2]): ...  # noqa

from typing import Literal
def f(x: Literal[1] | Literal[2]): ...  # noqa: PYI030

from typing import Literal
def f(x: Literal[1] | Literal[2]): ...  # NOQA: PYI030

from typing import Literal
def f(x: Literal[1] | Literal[2]): ...  # noqa: E501
