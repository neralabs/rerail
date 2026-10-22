import pytest


@pytest.fixture()
def a():
    pass

import pytest


@pytest.fixture()  # noqa
def a():
    pass

import pytest


@pytest.fixture()  # noqa: PT001
def a():
    pass

import pytest


@pytest.fixture()  # NOQA: PT001
def a():
    pass

import pytest


@pytest.fixture()  # noqa: E501
def a():
    pass
