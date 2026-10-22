from typing import Union
a: Union[(int, int)]
b: Union[(int, int),]
c: Union[((int, int))]
d: Union[(int, int), str]
e: Union[(int, int), (int, int)]
f: Union[(int, str), int]
g: Union[  # c
    (int, int)
]
a = Union[(int, int)]
b = Union[(int, int),]
c = Union[((int, int))]
d = Union[(int, int), str]
e = Union[(int, int), (int, int)]
f = Union[(int, str), int]
g = Union[((int, int)), str]
h = Union[(((int, int)),)]
