import enum
class E(enum.Enum):
    A = 1
    B = 1

import enum
class E(enum.Enum):
    A = 1
    B = 1  # noqa

import enum
class E(enum.Enum):
    A = 1
    B = 1  # noqa: PIE796

import enum
class E(enum.Enum):
    A = 1
    B = 1  # NOQA: PIE796

import enum
class E(enum.Enum):
    A = 1
    B = 1  # noqa: E501
