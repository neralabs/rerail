import pytest


@pytest.fixture("x")
def a():
    pass

import pytest


@pytest.fixture("x")  # noqa
def a():
    pass

import pytest


@pytest.fixture("x")  # noqa: PT002
def a():
    pass

import pytest


@pytest.fixture("x")  # NOQA: PT002
def a():
    pass

import pytest


@pytest.fixture("x")  # noqa: E501
def a():
    pass
