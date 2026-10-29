from enum import Enum
import enum
class A((Enum)):
    X = None
class B((Enum)[int]):
    Y = None
class C((enum.Flag)):
    Z = None
    W = int | None
class D((Base)):
    V = None
