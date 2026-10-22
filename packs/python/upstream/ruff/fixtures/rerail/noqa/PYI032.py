from typing import Any
class C:
    def __eq__(self, other: Any) -> bool: ...

from typing import Any
class C:
    def __eq__(self, other: Any) -> bool: ...  # noqa

from typing import Any
class C:
    def __eq__(self, other: Any) -> bool: ...  # noqa: PYI032

from typing import Any
class C:
    def __eq__(self, other: Any) -> bool: ...  # NOQA: PYI032

from typing import Any
class C:
    def __eq__(self, other: Any) -> bool: ...  # noqa: E501
