from typing import Optional, Union
import typing
import typing_extensions as te


def f1(a: Union[bool, int], b: Union[bool], c: Union[(bool, int)], d: Union[bool,], e: Union[(bool)]):
    pass


def f2(a: Optional[bool], b: Optional[bool, int], c: Optional[(bool)], d: Optional[(bool,)], e: Optional["bool"]):
    pass


def f3(a: typing.Union[int, bool], b: typing.Optional[bool], c: te.Optional[bool], d: te.Union[str, "bool"]):
    pass


def f4(a: int | bool, b: (bool | None), c: None | int | bool, d: int | str, e: bool & int, f: Union[int, Optional[bool]]):
    pass


def f5(a: Union[(bool, int), str], b: Union[((bool, int))], c: list[bool], d: dict[str, bool], e: "bool | None"):
    pass


def f6(a: Union[bool, int] | None, b: None | Optional[bool], c: typing.Optional[bool] | int, d: Optional[Optional[int]], e: Union[Union[bool, int]]):
    pass
