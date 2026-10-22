from typing import Literal

def f(x: Literal[None]): ...  

from typing import Literal

def f(x: Literal[None]): ...    # noqa

from typing import Literal

def f(x: Literal[None]): ...    # noqa: PYI061

from typing import Literal

def f(x: Literal[None]): ...    # NOQA: PYI061

from typing import Literal

def f(x: Literal[None]): ...    # noqa: E501
