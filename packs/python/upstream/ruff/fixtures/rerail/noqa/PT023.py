import pytest


@pytest.mark.foo()
def test_x(): ...

import pytest


@pytest.mark.foo()  # noqa
def test_x(): ...

import pytest


@pytest.mark.foo()  # noqa: PT023
def test_x(): ...

import pytest


@pytest.mark.foo()  # NOQA: PT023
def test_x(): ...

import pytest


@pytest.mark.foo()  # noqa: E501
def test_x(): ...
