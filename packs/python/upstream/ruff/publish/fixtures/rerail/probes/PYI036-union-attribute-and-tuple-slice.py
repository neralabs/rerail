import typing as t
from typing import Union
from types import TracebackType
class A:
    def __exit__(self, a: t.Union[None, type[BaseException]], b: Union[(BaseException, None)], c: t.Union[None, TracebackType]) -> None: ...
class B:
    def __exit__(self, a: type[BaseException, ] | None, b: Union[(BaseException, None),], c: Union[(TracebackType, int)]) -> None: ...
