import pytest


@pytest.mark.usefixtures("a")
@pytest.fixture()
def f(): ...

import pytest


@pytest.mark.usefixtures("a")  # noqa
@pytest.fixture()
def f(): ...

import pytest


@pytest.mark.usefixtures("a")  # noqa: PT025
@pytest.fixture()
def f(): ...

import pytest


@pytest.mark.usefixtures("a")  # NOQA: PT025
@pytest.fixture()
def f(): ...

import pytest


@pytest.mark.usefixtures("a")  # noqa: E501
@pytest.fixture()
def f(): ...
