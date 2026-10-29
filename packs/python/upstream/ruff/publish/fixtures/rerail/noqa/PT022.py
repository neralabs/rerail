import pytest


@pytest.fixture()
def f():
    x = 1
    yield x

import pytest


@pytest.fixture()
def f():
    x = 1
    yield x  # noqa

import pytest


@pytest.fixture()
def f():
    x = 1
    yield x  # noqa: PT022

import pytest


@pytest.fixture()
def f():
    x = 1
    yield x  # NOQA: PT022

import pytest


@pytest.fixture()
def f():
    x = 1
    yield x  # noqa: E501
