import enum
import functools
from enum import IntEnum


class Plain:
    @functools.cache
    def method(self):
        pass


class Direct(enum.Enum):
    @functools.cache
    def method(self):
        pass


class Base(IntEnum):
    pass


class Derived(Base):
    @functools.lru_cache
    def method(self):
        pass


class Deeper(Derived):
    @functools.lru_cache()
    def method(self):
        pass


class Unrelated:
    pass


class FromUnrelated(Unrelated):
    @functools.cache
    def method(self):
        pass


Alias = Base


class FromAlias(Alias):
    @functools.cache
    def method(self):
        pass
