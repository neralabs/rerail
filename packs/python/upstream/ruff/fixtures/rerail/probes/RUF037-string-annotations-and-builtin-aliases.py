import builtins
import collections as c
from collections import deque

a: "deque([], 3)" = None
b = c.deque(builtins.list())
c2 = c.deque(iterable=())
d = deque(f"{''}" "")
e = (deque)(set(), maxlen=2)


def shadow(list):
    return deque(list())


def stays_silent():
    return deque("a"), deque([1]), deque(maxlen=3), deque([], 1, 2), deque(*[[]])
