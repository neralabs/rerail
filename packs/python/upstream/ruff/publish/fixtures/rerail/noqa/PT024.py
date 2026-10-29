import pytest


@pytest.mark.asyncio
@pytest.fixture()
def f(): ...

import pytest


@pytest.mark.asyncio  # noqa
@pytest.fixture()
def f(): ...

import pytest


@pytest.mark.asyncio  # noqa: PT024
@pytest.fixture()
def f(): ...

import pytest


@pytest.mark.asyncio  # NOQA: PT024
@pytest.fixture()
def f(): ...

import pytest


@pytest.mark.asyncio  # noqa: E501
@pytest.fixture()
def f(): ...
