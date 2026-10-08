import os
import sys
# ruff: noqa: F401
x = 1  # ruff: noqa: F841


def f():
    l = 1
    unused = 2
    # flake8: noqa: E741
    return 3
