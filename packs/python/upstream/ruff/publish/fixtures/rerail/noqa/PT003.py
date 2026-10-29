import pytest


@pytest.fixture(scope="function")
def my_fixture(): ...

import pytest


@pytest.fixture(scope="function")  # noqa
def my_fixture(): ...

import pytest


@pytest.fixture(scope="function")  # noqa: PT003
def my_fixture(): ...

import pytest


@pytest.fixture(scope="function")  # NOQA: PT003
def my_fixture(): ...

import pytest


@pytest.fixture(scope="function")  # noqa: E501
def my_fixture(): ...
