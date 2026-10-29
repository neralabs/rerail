from __future__ import division
from typing import TYPE_CHECKING, Union

a: Union["int", str] = 1
if (TYPE_CHECKING):
    b: Union["int", str]


def f[T: Union["int", str]](x: T) -> T: ...


type X = Union["int", str]
