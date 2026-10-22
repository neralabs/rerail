import typing
from typing_extensions import overload as ovl


class A:
    def __init__(self):
        # comment
        pass

    @ovl
    def __init__(self, x): ...

    @__init__.deleter
    def __init__(self): ...

    @__init__.getter
    def __init__(self): ...

    @(typing.override)
    def __init__(self): ...

    def __init__(self): \
        """Doc."""

    def __init__(self): r'''Doc.'''

    def __init__(self):
        ("Doc."
         # c
         "More.")

    def __init__(self):
        b"Doc."

    def __init__(self):
        x: str = "Doc."

    class Inner:
        def __init__(self): ...

    class _Inner:
        def __init__(self): ...

    def method(self):
        class Local:
            def __init__(self): ...

        def __init__(self): ...

    if typing.TYPE_CHECKING:
        def __init__(self): ...


class _B:
    def __init__(self): ...


def __init__(self): ...
