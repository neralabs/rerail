import pytest


@pytest.mark.parametrize("a, b", [(1, 2)])
def test_x(a, b): ...

import pytest


@pytest.mark.parametrize("a, b", [(1, 2)])  # noqa
def test_x(a, b): ...

import pytest


@pytest.mark.parametrize("a, b", [(1, 2)])  # noqa: PT006
def test_x(a, b): ...

import pytest


@pytest.mark.parametrize("a, b", [(1, 2)])  # NOQA: PT006
def test_x(a, b): ...

import pytest


@pytest.mark.parametrize("a, b", [(1, 2)])  # noqa: E501
def test_x(a, b): ...
