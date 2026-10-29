import pytest
from pytest import mark
from pytest.mark import usefixtures


@pytest.mark.usefixtures()  # it's "empty" # really
def test_a(): ...


@mark.usefixtures  # bare attribute
def test_b(): ...


@usefixtures
def test_c(): ...  # a bare imported name is not reported


@usefixtures(  # empty call of an imported name
)
def test_d(): ...


@pytest.mark.usefixtures()
class TestE: ...


@(pytest.mark.usefixtures)
async def test_f(): ...
