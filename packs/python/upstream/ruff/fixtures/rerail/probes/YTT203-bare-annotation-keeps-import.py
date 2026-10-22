import sys

sys: int
sys.version_info[1] > 5


def f():
    sys: int = 3
    return sys.version_info[1] > 5
