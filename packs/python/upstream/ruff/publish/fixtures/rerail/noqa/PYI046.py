from typing import Protocol
def f():
    class _P(Protocol): ...

from typing import Protocol
def f():
    class _P(Protocol): ...  # noqa

from typing import Protocol
def f():
    class _P(Protocol): ...  # noqa: PYI046

from typing import Protocol
def f():
    class _P(Protocol): ...  # NOQA: PYI046

from typing import Protocol
def f():
    class _P(Protocol): ...  # noqa: E501
