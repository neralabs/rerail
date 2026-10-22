from typing import Generator

class A:
    def __iter__(self) -> Generator:
        "a", "b"
    def __iter__(self) -> Generator:
        ..., ...
    def __iter__(self) -> Generator:
        yield 1, 2
    def __iter__(self) -> Generator:
        x: int
    def __iter__(self) -> Generator:
        yield 1
        return None, None
    def __iter__(self) -> Generator:
        yield 1
        return (None,)
    def __iter__(self) -> Generator:
        "x",
