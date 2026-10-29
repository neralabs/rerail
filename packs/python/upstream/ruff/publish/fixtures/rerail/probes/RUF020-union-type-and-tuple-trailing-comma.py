from typing import Never, Union
import typing

a: list[int] | Never
b: list[int] | (typing.NoReturn) | str
c: Union[(Never, int),]
d = Union[(Never, int),]
e: Union[(Never, int)]
f = Union[(Never, int)]
