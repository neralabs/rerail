from typing import Union
a: "Union[type[int], type[str]]" ""
b: "type[int] | type[str]" ""
c: "Union[type[int], \x74ype[str]]"
d: "type[int] | type[str] | None" ""
e: "Union[type[int] | type[str], int]" ""
f: "type[int] | Union[type[str], type[bytes]]" ""
g: "list['type[int]' '|type[str]']"
