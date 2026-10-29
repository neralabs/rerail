def foo(arg: list[int] = list(range(10_000))) -> None: ...

def foo(arg: list[int] = list(range(10_000))) -> None: ...  # noqa

def foo(arg: list[int] = list(range(10_000))) -> None: ...  # noqa: PYI011

def foo(arg: list[int] = list(range(10_000))) -> None: ...  # NOQA: PYI011

def foo(arg: list[int] = list(range(10_000))) -> None: ...  # noqa: E501
