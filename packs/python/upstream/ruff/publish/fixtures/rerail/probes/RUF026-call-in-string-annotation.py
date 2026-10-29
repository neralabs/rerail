from typing import cast, List, TypeVar
from collections import defaultdict


def f(a: "defaultdict(default_factory=int)") -> "defaultdict(default_factory=list)":
    b: "List['defaultdict(default_factory=int)']" = 1
    c: "defaultdict" "(default_factory=int)" = 1
    d: "defaultdict(default_factory=\x69nt)" = 1
    e: """defaultdict(default_factory=int)""" = 1
    g = cast("defaultdict(default_factory=int)", a)
    T = TypeVar("T", bound="defaultdict(default_factory=int)")
    h: List["defaultdict(default_factory=int)"] = 1
    return defaultdict
