from enum import Enum, IntEnum


class Color(Enum):
    RED = "red"
    BLUE = "blue"


class Status(str, Enum):
    OPEN = "open"
    CLOSED = "closed"


class Level(IntEnum):
    LOW = 1
    HIGH = 2
