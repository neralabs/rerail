from typing import Iterator, Generator

class C:
    def __iter__(self) -> Generator[int, None, None]:  
        yield 1

from typing import Iterator, Generator

class C:
    def __iter__(self) -> Generator[int, None, None]:    # noqa
        yield 1

from typing import Iterator, Generator

class C:
    def __iter__(self) -> Generator[int, None, None]:    # noqa: PYI058
        yield 1

from typing import Iterator, Generator

class C:
    def __iter__(self) -> Generator[int, None, None]:    # NOQA: PYI058
        yield 1

from typing import Iterator, Generator

class C:
    def __iter__(self) -> Generator[int, None, None]:    # noqa: E501
        yield 1
