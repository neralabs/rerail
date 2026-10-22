import pytest
from typing import cast


def func():
    pass


x: "pytest.warns(UserWarning, func)" = 1
y = cast("pytest.deprecated_call(func)", 1)
z: "list['pytest.raises(ValueError, func=func)']" = []
w: "pytest.raises(ValueError)" = 1
v = "pytest.raises(ValueError, func)"


def test_a(a: "pytest.raises(ValueError, func)" = None) -> "pytest.raises(func, ValueError, func)":
    pass
