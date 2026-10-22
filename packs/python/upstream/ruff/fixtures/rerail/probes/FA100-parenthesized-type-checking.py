from typing import TYPE_CHECKING, Optional, Union
import typing

if (TYPE_CHECKING):
    a: Optional["Foo"]
if ((typing.TYPE_CHECKING)):
    b: Union[int, "Foo"]
if not TYPE_CHECKING:
    c: Optional["Foo"]
if TYPE_CHECKING and True:
    d: Optional["Foo"]
