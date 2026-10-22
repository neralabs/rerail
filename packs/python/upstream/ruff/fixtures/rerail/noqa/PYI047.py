from typing import TypeAlias
def f():
    _A: TypeAlias = int

from typing import TypeAlias
def f():
    _A: TypeAlias = int  # noqa

from typing import TypeAlias
def f():
    _A: TypeAlias = int  # noqa: PYI047

from typing import TypeAlias
def f():
    _A: TypeAlias = int  # NOQA: PYI047

from typing import TypeAlias
def f():
    _A: TypeAlias = int  # noqa: E501
