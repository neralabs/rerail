from typing import TypedDict
def f():
    class _T(TypedDict):
        a: int

from typing import TypedDict
def f():
    class _T(TypedDict):  # noqa
        a: int

from typing import TypedDict
def f():
    class _T(TypedDict):  # noqa: PYI049
        a: int

from typing import TypedDict
def f():
    class _T(TypedDict):  # NOQA: PYI049
        a: int

from typing import TypedDict
def f():
    class _T(TypedDict):  # noqa: E501
        a: int
