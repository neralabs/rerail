import pytest


@pytest.mark.parametrize("a", (1, 2))
def test_x(a): ...

import pytest


@pytest.mark.parametrize("a", (1, 2))  # noqa
def test_x(a): ...

import pytest


@pytest.mark.parametrize("a", (1, 2))  # noqa: PT007
def test_x(a): ...

import pytest


@pytest.mark.parametrize("a", (1, 2))  # NOQA: PT007
def test_x(a): ...

import pytest


@pytest.mark.parametrize("a", (1, 2))  # noqa: E501
def test_x(a): ...
