__all__ = ("A",) + ("B",) + ["C"]


class A:
    def __eq__(self, o): ...


class B:
    def __eq__(self, o): ...


class C:
    def __eq__(self, o): ...


class D:
    def __eq__(self, o): ...
