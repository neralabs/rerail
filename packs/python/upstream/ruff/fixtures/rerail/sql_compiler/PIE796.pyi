# Positive and negative stub witnesses for relational compiler parity.
from enum import Enum, auto


class DuplicateValues(Enum):
    FIRST = 1
    SECOND = 1
    UNKNOWN_FIRST = ...
    UNKNOWN_SECOND = ...


class DistinctValues(Enum):
    FIRST = 1
    SECOND = 2
    AUTOMATIC_FIRST = auto()
    AUTOMATIC_SECOND = auto()
