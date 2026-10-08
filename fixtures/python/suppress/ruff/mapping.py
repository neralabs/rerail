#!/usr/bin/env python3
# noqa: INP001
"""A module docstring
over lines."""
import os
import sys
from os import (  # noqa: F401
    path,
    sep,
)
from json import (
    dumps,  # noqa: F401
    loads,
)  # noqa: F401


def f():
    a = """one
    two"""  # noqa: F841
    b = """one
    two"""
    c = f"""{a}
    {b}"""  # noqa: F841
    d = f"{a}"  # noqa: F841
    e = 1 + \
        2  # noqa: F841
    g = 1 + \
        2 + \
        3  # noqa: F841
    h = 1 + \
        2  # noqa: F841
    i = f"{
        a
    }"  # noqa: F841
    j = f"""{f'''x
    {a}'''}"""  # noqa: F841
    k = (1,
         2)  # noqa: F841
    m = "\d" \
        "\w"  # noqa: W605
    n = """\d
    """  # noqa: W605
    return b


__all__ = [  # noqa: F822
    "missing",
    "absent",
]
__all__ += [
    "gone",  # noqa: F822
]
