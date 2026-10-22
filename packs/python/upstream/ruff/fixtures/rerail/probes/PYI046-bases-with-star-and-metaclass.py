import typing
from typing import Generic, Protocol, TypeVar

T = TypeVar("T")
bases = ()


class Base: ...


class M(type): ...


class _A(Generic[T], Protocol):
    x: T


class _B(Base, typing.Protocol[T]):
    x: T


class _C(*bases, Protocol):
    x: int


class _D(Protocol, metaclass=M):
    x: int


class _E(Base, metaclass=Protocol):
    x: int


class _F(*[Protocol]):
    x: int
