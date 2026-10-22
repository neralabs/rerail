import pytest


@pytest.mark.usefixtures()
def test_x(): ...

import pytest


@pytest.mark.usefixtures()  # noqa
def test_x(): ...

import pytest


@pytest.mark.usefixtures()  # noqa: PT026
def test_x(): ...

import pytest


@pytest.mark.usefixtures()  # NOQA: PT026
def test_x(): ...

import pytest


@pytest.mark.usefixtures()  # noqa: E501
def test_x(): ...
