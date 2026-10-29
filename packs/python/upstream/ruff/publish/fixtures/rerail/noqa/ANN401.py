from typing import Any


def f(x: Any) -> None:
    pass

from typing import Any


def f(x: Any) -> None:  # noqa
    pass

from typing import Any


def f(x: Any) -> None:  # noqa: ANN401
    pass

from typing import Any


def f(x: Any) -> None:  # NOQA: ANN401
    pass

from typing import Any


def f(x: Any) -> None:  # noqa: E501
    pass
