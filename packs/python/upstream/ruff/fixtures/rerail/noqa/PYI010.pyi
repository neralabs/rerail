def foo() -> int:
    return 1

def foo() -> int:
    return 1  # noqa

def foo() -> int:
    return 1  # noqa: PYI010

def foo() -> int:
    return 1  # NOQA: PYI010

def foo() -> int:
    return 1  # noqa: E501
