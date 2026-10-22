import pytest


@pytest.fixture
@pytest.yield_fixture
def a():
    pass


@pytest.yield_fixture
@pytest.fixture
def b():
    pass


@pytest.yield_fixture
@pytest.yield_fixture
def c():
    pass


@pytest.mark.foo
@pytest.yield_fixture()
@pytest.mark.bar
def d():
    pass


@pytest.fixture()
@pytest.mark.foo
@pytest.yield_fixture()
def e():
    pass
