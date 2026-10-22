from typing import Iterable
class C:
    def __iter__(self) -> Iterable[int]: ...

from typing import Iterable
class C:
    def __iter__(self) -> Iterable[int]: ...  # noqa

from typing import Iterable
class C:
    def __iter__(self) -> Iterable[int]: ...  # noqa: PYI045

from typing import Iterable
class C:
    def __iter__(self) -> Iterable[int]: ...  # NOQA: PYI045

from typing import Iterable
class C:
    def __iter__(self) -> Iterable[int]: ...  # noqa: E501
