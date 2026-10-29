from typing import TypeVar
_S = TypeVar("_S", bound="C")
class C:
    def m(self: _S) -> _S: ...

from typing import TypeVar
_S = TypeVar("_S", bound="C")
class C:
    def m(self: _S) -> _S: ...  # noqa

from typing import TypeVar
_S = TypeVar("_S", bound="C")
class C:
    def m(self: _S) -> _S: ...  # noqa: PYI019

from typing import TypeVar
_S = TypeVar("_S", bound="C")
class C:
    def m(self: _S) -> _S: ...  # NOQA: PYI019

from typing import TypeVar
_S = TypeVar("_S", bound="C")
class C:
    def m(self: _S) -> _S: ...  # noqa: E501
