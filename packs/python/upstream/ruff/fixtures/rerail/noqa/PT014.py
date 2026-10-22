import pytest


@pytest.mark.parametrize("x", [1, 1])
def test_x(x): ...

import pytest


@pytest.mark.parametrize("x", [1, 1])  # noqa
def test_x(x): ...

import pytest


@pytest.mark.parametrize("x", [1, 1])  # noqa: PT014
def test_x(x): ...

import pytest


@pytest.mark.parametrize("x", [1, 1])  # NOQA: PT014
def test_x(x): ...

import pytest


@pytest.mark.parametrize("x", [1, 1])  # noqa: E501
def test_x(x): ...
