from typing import Annotated, Literal, Union


def f(
    a: Annotated[(int, 1)] = None,
    b: Literal[(None, 1)] = None,
    c: Literal[(1, 2)] = None,
    d: Union[(int, str)] = None,
    e: Union[(int, None)] = None,
    f: Literal[(None)] = None,
    g: Annotated[(int)] = None,
    h: Annotated[((int, 1))] = None,
): ...
