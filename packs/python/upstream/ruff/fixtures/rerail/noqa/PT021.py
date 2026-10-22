import pytest


@pytest.fixture()
def f(request):
    request.addfinalizer(lambda: None)

import pytest


@pytest.fixture()
def f(request):
    request.addfinalizer(lambda: None)  # noqa

import pytest


@pytest.fixture()
def f(request):
    request.addfinalizer(lambda: None)  # noqa: PT021

import pytest


@pytest.fixture()
def f(request):
    request.addfinalizer(lambda: None)  # NOQA: PT021

import pytest


@pytest.fixture()
def f(request):
    request.addfinalizer(lambda: None)  # noqa: E501
