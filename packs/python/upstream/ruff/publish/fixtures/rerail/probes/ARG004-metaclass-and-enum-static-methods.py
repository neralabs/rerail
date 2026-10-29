import enum
from dataclasses import dataclass
from typing import NamedTuple, Protocol


@dataclass
class D:
    x: int

    def m(self, a):
        return self.x

    def __post_init__(self, init_only):
        pass


class P(Protocol):
    def m(self, a) -> int: ...

    def n(self, a) -> int:
        """Doc."""

    def o(self, a) -> int:
        return 0


class Meta(type):
    def __new__(mcs, name, bases, ns, **kw):
        return super().__new__(mcs, name, bases, ns)

    def __prepare__(mcs, name, bases, **kw):
        return {}

    def __call__(cls, *args, **kwargs):
        return super().__call__()

    def method(cls, a):
        print(1)


class E(enum.Enum):
    A = 1

    def _generate_next_value_(name, start, count, last_values):
        return name

    @staticmethod
    def _gen(name, start, count, last_values):
        return name


class N(NamedTuple):
    a: int

    def m(self, b):
        return self.a
