@dataclass
class A:
    def __post_init__(self, x=1): ...


def f():
    @dataclass
    class B:
        def __post_init__(self, y=2): ...

    return B


from dataclasses import dataclass
