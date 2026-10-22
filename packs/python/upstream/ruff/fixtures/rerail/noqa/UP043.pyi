from collections.abc import Generator

def f() -> Generator[int, None, None]: ...

from collections.abc import Generator

def f() -> Generator[int, None, None]: ...  # noqa

from collections.abc import Generator

def f() -> Generator[int, None, None]: ...  # noqa: UP043

from collections.abc import Generator

def f() -> Generator[int, None, None]: ...  # NOQA: UP043

from collections.abc import Generator

def f() -> Generator[int, None, None]: ...  # noqa: E501
