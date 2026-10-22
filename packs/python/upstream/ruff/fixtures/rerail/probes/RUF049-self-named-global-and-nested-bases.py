from dataclasses import dataclass
from enum import Enum


class E(Enum): ...


@dataclass
class E(E): ...


@dataclass
class Enum(Enum): ...


def g():
    global E

    @dataclass
    class Y(E): ...


class Outer:
    class Inner(Enum): ...
    class Gone(Enum): ...
    Gone = 1


@dataclass(repr="#")  # it's)
class V(Outer.Inner): ...


@dataclass
class W(Outer.Gone): ...


@dataclass \
    (frozen=True)
class X(Enum \
        [int]): ...
