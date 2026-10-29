from typing import Generic, TypeVar
T = TypeVar("T")

class C(Generic[T], list):  
    ...

from typing import Generic, TypeVar
T = TypeVar("T")

class C(Generic[T], list):    # noqa
    ...

from typing import Generic, TypeVar
T = TypeVar("T")

class C(Generic[T], list):    # noqa: PYI059
    ...

from typing import Generic, TypeVar
T = TypeVar("T")

class C(Generic[T], list):    # NOQA: PYI059
    ...

from typing import Generic, TypeVar
T = TypeVar("T")

class C(Generic[T], list):    # noqa: E501
    ...
