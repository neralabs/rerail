from typing import Union, Annotated


class A:
    a: Union[((int, str))] = []
    b: Annotated[((int, 1))] = []
    c: Union[(int, list)] = []
    d: Annotated[(list, 1)] = []
    e: Union[(int,)] = []
    f: Annotated[(int,)] = []
