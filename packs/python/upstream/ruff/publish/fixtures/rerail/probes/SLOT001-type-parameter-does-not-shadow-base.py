from typing import Tuple


class A[tuple](tuple):  # bases resolve outside the type-parameter scope
    pass


class B[T](tuple[T, ...]):
    pass


class C[Tuple](Tuple):  # an imported name is resolved where it is read
    pass


class D(Tuple[int][0]):  # only one subscript level is unwrapped
    pass
