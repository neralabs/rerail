from typing import TypeVar
def f():
    _T = TypeVar("_T")

from typing import TypeVar
def f():
    _T = TypeVar("_T")  # noqa

from typing import TypeVar
def f():
    _T = TypeVar("_T")  # noqa: PYI018

from typing import TypeVar
def f():
    _T = TypeVar("_T")  # NOQA: PYI018

from typing import TypeVar
def f():
    _T = TypeVar("_T")  # noqa: E501
