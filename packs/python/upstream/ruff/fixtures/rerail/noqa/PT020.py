import pytest


@pytest.yield_fixture
def f(): ...

import pytest


@pytest.yield_fixture  # noqa
def f(): ...

import pytest


@pytest.yield_fixture  # noqa: PT020
def f(): ...

import pytest


@pytest.yield_fixture  # NOQA: PT020
def f(): ...

import pytest


@pytest.yield_fixture  # noqa: E501
def f(): ...
