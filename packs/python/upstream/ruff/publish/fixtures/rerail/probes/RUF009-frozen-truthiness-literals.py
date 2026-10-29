from dataclasses import dataclass


@dataclass(frozen=-1)
class Negative:
    x: int = 1


@dataclass(frozen=[1])
class NonEmptyList:
    x: int = 1


@dataclass(frozen=list("a"))
class ListCall:
    x: int = 1


@dataclass(frozen=f"{x}")
class OpaqueFString:
    x: int = 1


@dataclass
class Holder:
    a: Negative = Negative()
    b: NonEmptyList = NonEmptyList()
    c: ListCall = ListCall()
    d: OpaqueFString = OpaqueFString()
    e: tuple = (tuple)[int]()
    f: list = frozendict()
