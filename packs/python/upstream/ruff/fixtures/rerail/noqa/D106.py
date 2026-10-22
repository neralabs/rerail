class A:
    """Doc."""

    class B: ...

class A:
    """Doc."""

    class B: ...  # noqa

class A:
    """Doc."""

    class B: ...  # noqa: D106

class A:
    """Doc."""

    class B: ...  # NOQA: D106

class A:
    """Doc."""

    class B: ...  # noqa: E501
