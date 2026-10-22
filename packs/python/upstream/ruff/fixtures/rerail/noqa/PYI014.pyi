def foo(arg=bar()) -> None: ...

def foo(arg=bar()) -> None: ...  # noqa

def foo(arg=bar()) -> None: ...  # noqa: PYI014

def foo(arg=bar()) -> None: ...  # NOQA: PYI014

def foo(arg=bar()) -> None: ...  # noqa: E501
