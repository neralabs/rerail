import enum
from enum import Enum, IntEnum


class A(Enum):
    X = 1
    Y = 1.0
    Z = True
    W = "a"
    V = 'a'
    U = "a" ""
    T = (1, 2)
    S = (1, 2)
    R = x
    Q = x
    P = enum.auto()
    O = enum.auto()
    N = -1
    M = -1
    L = 1 + 1
    K = 2
    J = 0x2
    I = b"a"
    H = None
    G = None


class B(IntEnum):
    X = 1
    Y = 1


class C(enum.Enum):
    X = "\x61"
    Y = "a"
    Z = 1j
    W = 1.0j


class D:
    X = 1
    Y = 1


class E(Enum):
    X = 1
    Y: int = 1
    Z = (1, 2.0)
    W = (1, 2)
